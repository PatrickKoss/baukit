---
name: baukit-security-testing
description: Add security tests to a Baukit product: cargo-fuzz targets for parsers of untrusted input, proptest properties, auth negative tests, cross-tenant tests, SSRF tests through baukit-egress, and body and upload limit tests. Use when a threat-model row needs a test, when adding a parser, decoder, signed value, route group, integration or upload, or when changing token verification.
---

# Add security tests to a Baukit product

Each test proves one control from `docs/threat-model.md`. Name the row ID in the test module or file so the table can point at it. Tests that need PostgreSQL, Keycloak or Redis use `baukit-test` containers and run with `-- --include-ignored`.

Anti-patterns that apply everywhere:

- asserting only the status code (`200`, `401`); also assert the body class and that no side effect happened;
- `sleep` to wait for expiry or a job; set the clock or mint an already expired claim;
- mocking the token verifier, the egress client or the database; the test must run the real control;
- `let _ = ...` on the result under test, or `#[ignore]` without the container reason.

## Fuzz parsers of untrusted input

Applies to code that decodes bytes from a client, peer, webhook, file import or third party: statement and file parsers, custom header parsers, envelope and batch decoders. Not to serde derives of simple JSON structs.

Layout, outside the backend workspace so nightly never touches the main build:

```text
backend/fuzz/Cargo.toml          [workspace] members = ["."], cargo-fuzz = true metadata
backend/fuzz/fuzz_targets/<name>.rs
backend/fuzz/corpus/<name>/      seeded from fixtures, gitignored
backend/fuzz/artifacts/<name>/   crashes, gitignored
scripts/fuzz.sh <name> <seconds>
```

```toml
[package]
name = "<product>-fuzz"
version = "0.0.0"
publish = false
edition = "2024"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
<product>-domain = { path = "../crates/<product>-domain" }

[workspace]
members = ["."]

[[bin]]
name = "import_csv"
path = "fuzz_targets/import_csv.rs"
test = false
doc = false
bench = false
```

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = product_domain::import::parse_csv(data) {
        assert!(parsed.rows.len() <= product_domain::import::MAX_ROWS);
    }
});
```

```sh
#!/bin/sh
set -eu
cd "$(dirname -- "$0")/.."
target=${1:?usage: fuzz.sh TARGET SECONDS}
seconds=${2:?usage: fuzz.sh TARGET SECONDS}
case "$seconds" in ''|*[!0-9]*) exit 2 ;; esac
mkdir -p "backend/fuzz/corpus/$target"
cp fixtures/import/*.csv "backend/fuzz/corpus/$target/"
cargo +nightly fuzz run --fuzz-dir backend/fuzz "$target" -- -max_total_time="$seconds" -max_len=16384
```

Run 60 seconds per target locally and in the gate, longer on a schedule. A crash leaves `backend/fuzz/artifacts/<target>/crash-*`: reproduce with `cargo +nightly fuzz run --fuzz-dir backend/fuzz <target> <file>`, copy the input into a regular unit test under the domain crate's fixtures, fix, and keep that test. Never commit the artifacts directory.

Anti-patterns: a target that discards the result without any invariant when one exists, unbounded `-max_len`, a fuzz run with no time limit in the gate, seeding the corpus from production data.

## Property tests

Applies to pure logic with an inverse, a merge, a signature or an owner key. Use `proptest` as a dev-dependency.

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn encode_decode_roundtrips(value in any_note()) {
        prop_assert_eq!(decode(&encode(&value))?, value);
    }

    #[test]
    fn applying_a_batch_twice_equals_applying_it_once(batch in any_batch()) {
        let once = apply(State::default(), &batch);
        prop_assert_eq!(apply(once.clone(), &batch), once);
    }

    #[test]
    fn a_batch_never_writes_under_another_owner(owner in any_owner(), batch in any_batch()) {
        let state = apply_as(owner, State::default(), &batch);
        prop_assert!(state.rows().all(|row| row.owner == owner));
    }

    #[test]
    fn any_single_byte_change_of_a_signed_value_is_rejected(
        payload in any_payload(),
        index in any::<prop::sample::Index>(),
        flip in 1u8..=255,
    ) {
        let mut signed = sign(&KEY, &payload).into_bytes();
        let position = index.index(signed.len());
        if let Some(byte) = signed.get_mut(position) {
            *byte ^= flip;
        }
        prop_assert!(verify(&KEY, &signed).is_err());
    }
}
```

Anti-patterns: strategies so narrow they only produce the happy path, properties that re-implement the code under test, catching the failure and returning `Ok`.

## Auth negative tests

Applies to every product route group and the MCP endpoint, and whenever the verifier profile, IdP or token storage changes. Use the real verifier against `baukit_test::jwt::MockOidcServer` (or the Keycloak container) and call the router.

Cases, each one request, each expecting `401` with the standard error envelope and no handler side effect:

| Case | How to build it |
|---|---|
| expired | `exp` in the past by more than the skew |
| wrong `iss` | `.issuer("https://other.example")` |
| wrong `aud` | `.audience("other-api")` |
| `alg: none` | `baukit_test::jwt::unsigned_token` |
| HS256 with the public key as secret | `hs256_token(public_key_pem, &claims)` |
| unknown `kid` | `rs256_token_with_key_id` with a kid absent from JWKS |
| tampered payload | valid token, change one claim, keep the signature |
| missing `azp` (Clerk) | omit `azp` from an otherwise valid token |

```rust
#[tokio::test]
async fn tokens_signed_hs256_with_the_public_key_are_rejected() -> Result<(), Box<dyn Error>> {
    let oidc = MockOidcServer::start().await?;
    let app = test_app(&oidc).await?;
    let token = hs256_token(PUBLIC_KEY_PEM, &oidc.claims("user-a"))?;

    let response = app.get("/v1/notes").bearer(&token).send().await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.error_code()?, "unauthenticated");
    assert_eq!(app.notes_count().await?, 0);
    Ok(())
}
```

Anti-patterns: injecting a fake `Principal` instead of a token, one test that only checks a missing header, accepting `403` where `401` is the contract.

## Cross-tenant tests

Applies to every route that takes a resource ID. Repositories take `(owner_id, id)`; a row owned by someone else is a missing row and maps to `ApiError::not_found` from `baukit-http`, so the response is `404`, never `403`, and the body does not reveal the row exists.

```rust
#[tokio::test]
#[ignore = "needs Docker PostgreSQL"]
async fn user_b_cannot_read_update_or_delete_user_a_notes() -> Result<(), Box<dyn Error>> {
    let app = test_app().await?;
    let note = app.as_user("user-a").create_note("private").await?;
    let b = app.as_user("user-b");

    assert_eq!(b.get(&format!("/v1/notes/{}", note.id)).await?.status(), 404);
    assert_eq!(b.patch(&format!("/v1/notes/{}", note.id), json!({"title": "x"})).await?.status(), 404);
    assert_eq!(b.delete(&format!("/v1/notes/{}", note.id)).await?.status(), 404);
    assert_eq!(b.get(&format!("/v1/notes/{}", Uuid::now_v7())).await?.body(), b.get(&format!("/v1/notes/{}", note.id)).await?.body());

    let unchanged = app.as_user("user-a").get_note(note.id).await?;
    assert_eq!(unchanged.title, "private");
    Ok(())
}
```

Cover list endpoints and sync pulls too: B's list never contains A's rows. For organizations, repeat with org A and org B.

Anti-patterns: testing only GET, checking status without checking A's row is unchanged, seeding both users with the same ID.

## SSRF through baukit-egress

Applies to every outbound call. Build clients with `baukit_egress::GuardedClient`, never `reqwest::Client::new`. When a URL comes from a user, tenant, webhook registration or third-party response, test that private and loopback answers, address literals and redirects are refused before any connection.

```rust
#[tokio::test]
async fn webhook_targets_resolving_to_loopback_are_blocked() -> Result<(), Box<dyn Error>> {
    let resolver = Arc::new(StaticResolver::new().with_host("hooks.test", [IpAddr::V4(Ipv4Addr::LOCALHOST)]));
    let client = GuardedClient::with_resolver(resolver, EgressOptions::default())?;

    let error = client
        .execute(EgressRequest::post(Url::parse("https://hooks.test/deliver")?))
        .await
        .expect_err("loopback must be blocked");

    assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    Ok(())
}
```

Also cover `http://169.254.169.254/`, `http://[::1]/`, and a 302 to a private address: the product must not follow it. Anti-pattern: asserting on an error from a host that simply does not resolve, which passes without the policy.

## Body, upload and size limits

Applies to every route that accepts a body, file, batch or list. `baukit-http` sets a global body limit and per-route limits; `baukit-core::limits` and the product's row caps bound counts.

- One byte over the limit returns `413` with the body-too-large error class, and nothing is stored.
- Exactly the limit is accepted.
- Batches one item over the cap are rejected whole.
- Uploads with a wrong declared content type or a size that differs from the bytes sent are rejected.

```rust
#[tokio::test]
async fn attachments_over_the_route_limit_are_rejected_without_storing() -> Result<(), Box<dyn Error>> {
    let app = test_app().await?;
    let body = vec![0u8; ATTACHMENT_LIMIT_BYTES + 1];

    let response = app.as_user("user-a").post_bytes("/v1/attachments", body).await?;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(app.attachment_count().await?, 0);
    Ok(())
}
```

Anti-pattern: testing a far-oversized body only; the off-by-one at the limit is where the bugs are.
