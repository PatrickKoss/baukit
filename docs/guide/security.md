# Secure a product

This page is for product developers who want to know which security checks a generated backend
already runs, what they must add for their own routes and data, and how far baukit's own proofs
go. At the end you know which file holds each check, how to extend it for a new route, and which
claims you cannot lean on.

The paths below are those of a product rendered with
`baukit new <name> --backend --web --auth oidc --mcp`. `<app>` stands for the product name.

## What every product inherits

### Cross-tenant and invalid-token suites

`backend/tests/conformance.rs` has the test `item_routes_deny_other_tenants_and_invalid_tokens`.
It seeds an item as Alice, then runs two `baukit_test` suites over the product's OpenAPI document
against the composed router:

- `assert_cross_tenant_denied` calls every operation as Bob with Alice's ids. Each operation with a
  path parameter must answer 403 or 404, and no body may contain the leak marker. With
  `.owner(token)`, Alice re-reads each resource to prove the denied request changed nothing.
- `assert_auth_negative_suite` sends every `InvalidCredential` to every secured operation and
  expects 401: missing, expired, other audience, unknown key, `alg: none`, wrong issuer, not yet
  valid, HS256 keyed with the public JWK, random `kid`, a duplicate `Authorization` header and a
  `Basic` header. It also sends one valid token per operation, which must not get 401.

A new route with a path parameter and no fixture value fails the cross-tenant suite until the
test names a value or an exclusion with a reason. An operation documented without security fails
the auth suite until the case declares it with `.public(method, path, reason)`. The test is marked
`#[ignore = "requires Docker PostgreSQL"]`, so it runs with `cargo test -- --include-ignored`.

The [`baukit-test` README](../../rust/crates/baukit-test/README.md) documents both suites.

### Composed-router tests

`backend/tests/composed/` runs against the router `compose_api` returns, the one the binary serves:

- `layer_coverage.rs`: every served route is rate limited, body capped and sends the security
  headers. Routes merged outside the OpenAPI document are listed in `support.rs`, and a late
  router missing from that list fails the test.
- `limiter_socket.rs`: the limiter keys on the real socket peer when the app is served the way the
  binary serves it.
- `auth_order.rs`: the IP limiter runs before token verification, so junk bearers cannot buy
  verifier work.
- `router_proptest.rs`: random requests never succeed on secured paths, never return a 5xx and
  always carry the security headers.

### A fuzz target

`backend/fuzz/fuzz_targets/item_body.rs` feeds bytes to `SaveItemRequest` and asserts that an
accepted item name is non-empty and trimmed. `backend/fuzz` is its own workspace, because
cargo-fuzz needs nightly. The strict profile's `scripts/quality-gate.sh` runs every target for
`BAUKIT_FUZZ_SECONDS` (default 60). The standard profile's CI does not run the fuzz targets, so
run them yourself with `cargo +nightly fuzz run --fuzz-dir backend/fuzz item_body`.

### `baukit check` rules

CI runs `baukit check ... patterns ... fuzz-lockfile security --root .`, which reads
`baukit-check.toml`.

- `patterns` runs the ast-grep rules in `lint/ast-grep/rules`. Rules with severity `error` fail
  the check: SQL built with `format!` or spliced strings, a raw HTTP client outside
  `baukit-egress`, `Debug` on secret types, `dangerouslySetInnerHTML`, double casts, lying type
  predicates, response casts and tokens in web storage.
- `security` fails when `axum::serve` runs without `into_make_service_with_connect_info`, when a
  bin crate merges a router without `rate_limited(` or `protect_routes(`, when `opsService.type`
  in any YAML file is not `ClusterIP`, and when a config lists a `null` CORS origin or a plain
  `http` one outside a file whose path contains `local`.

The owner-in-WHERE rule, `rust-sql-unscoped-by-id`, has severity `hint`. `baukit check` prints it
only with `--verbose` and never fails on it. The cross-tenant suite is the check that catches a
query filtered by id alone.

### Supply chain and workflows

`scripts/repo-lint.sh` has one step per tool, and both profiles run them:

- `workflow-lint`: actionlint and zizmor over `.github/workflows`, configured by `zizmor.yml`.
- `secret-scan`: gitleaks over the git history, configured by `.gitleaks.toml`.
- `osv`: osv-scanner over the lockfiles, configured by `osv-scanner.toml`.
- `deny`: cargo-deny with `backend/deny.toml`.

### The ops port stays inside the cluster

The ops listener serves metrics and admin endpoints on its own port, see
[ADR 0011](../adr/0011-separate-api-and-ops-listeners.md). The chart schema in
`deploy/chart/baukit-app/values.schema.json` accepts only `ClusterIP` for `opsService.type` and
`workerOpsService.type`. The `security` rule checks the product's `deploy/values.yaml` too.

## What the product must add

### A route and its second-principal test

Repository queries on tenant data take the owner and the id together:
`WHERE owner_id = $1 AND id = $2`. When you add a route that takes an id, add its path parameter
to the `OperationFixtures` in `conformance.rs` with a value the first principal owns, and a body
with `.body(method, path, json)` for writes. If the route acts only on the caller, exclude it
with a reason, as the template does for `DELETE /me`. Organization routes need a second run with
two organizations.

### A fuzz target with an oracle

Add a target under `backend/fuzz/fuzz_targets/` and a `[[bin]]` entry in `backend/fuzz/Cargo.toml`
for every parser of untrusted bytes: imports, custom headers, envelopes. The target asserts an
invariant on accepted input, as `item_body.rs` does. A target that only checks for panics finds
less. Seed `backend/fuzz/corpus/<target>/` from fixtures, never from production data.

### A property test

Use `proptest` for Rust logic with an inverse, a merge or an owner key, and fast-check in a
`*.property.test.ts` file for TypeScript clients. `router_proptest.rs` is the template's example.
The `baukit-security-testing` skill has patterns for round trips, idempotent batches and signature
tampering.

### A threat model

The `baukit-threat-model` agent skill writes `docs/threat-model.md` for the product: data classes,
trust boundaries and one row per control. The `baukit-security-testing` skill then adds a test per
row and names the row ID in the test. Baukit's own [threat model](../threat-model.md) shows the
row format. See the [agent skills guide](agent-skills.md) to install them.

### Pull SQL conformance

Pull SQL for sync tables is product code. `baukit_test::check_pull_conformance` drives writes
that stamp several rows with one revision across two tables, overwrites rows while a pull pages,
pages one row per table at a time and compares the client replica with the source:

```rust
pub async fn check_pull_conformance<A: PullConformanceAdapter>(
    adapter: &A,
) -> Result<(), PullConformanceError>
```

Source: `rust/crates/baukit-test/src/pull_conformance.rs`. The adapter implements
`create_owner`, `write(owner, table, rows)` and `pull(owner, after, limit)` against the product's
real tables. `assert_pull_conformance` panics with the violation instead. See
[data and sync](data-and-sync.md) for the pull window.

### Fenced writes after erasure

Every product write for an authenticated subject runs inside a transaction that holds the subject
lock and passed the erasure fence. `PostgresErasureStore::guard_subject` does both:

```rust
pub async fn guard_subject(
    &self,
    transaction: &mut Transaction<'_, Postgres>,
    subject: &str,
    authenticated_at: Option<Timestamp>,
) -> Result<(), ErasureError>
```

Source: `rust/crates/baukit-erasure/src/store.rs`. It returns `ErasureError::ProfileErased` for a
session the fence rejects. `begin_guarded_write(subject, authenticated_at)` opens the transaction
and calls it for you. The template's `backend/crates/<app>-postgres/src/lib.rs` calls
`guard_subject` before it resolves the user. See [privacy](privacy.md).

## What baukit proves, and where each proof stops

[ADR 0027](../adr/0027-verification-strategy.md) assigns one tool per property class, and each
tool runs as a `scripts/ci-gate.sh` step. [Verification targets](../verification.md) lists every
harness, twin and model with its bounds. A proof covers only what its harness or model states.

### Fuzz targets

`rust/fuzz/fuzz_targets` holds targets for the decoders of untrusted input: `egress_destination`,
`event_envelope`, `expo_responses`, `http_headers`, `identity_deletion_payload`, `idp_delete_url`,
`jwks_parse`, `jwt_verify`, `mcp_jsonrpc`, `media_grant_verify`, `oauth_state_parse`,
`receipt_verify`, `session_cookie`, `suite_event_decode`, `suite_return_url`, `suite_ticket`,
`sync_hlc`, `webhook_verify` and `xff_resolve`. The `fuzz` step runs each for 60 seconds, and a
nightly run uses 1800 seconds. A clean run means no crash was found in that time. It does not
mean none exists.

### TLA+ models

`models/` holds protocol models that TLC checks over every interleaving of a small instance. Each
`<Name>_current.cfg` reproduces a bug in the code before its fix and must still report it:

| Model | What the `_current` configs reproduce |
|---|---|
| SyncPull | a page cut inside one revision loses rows (F-06), and a cursor taken across tables under READ COMMITTED skips a row (F-07) |
| JobLease | a stalled attempt whose lease expired still writes the job result |
| BffSessionFamily | a refresh rolled back after the IdP rotated the token logs out an honest browser |
| ErasureSaga | an erasure completes while product rows survive (F-40), and a lock-free admission lets a fenced session write (F-42) |
| ClientAccountSwitch | an erase signal deletes another account's partition (F-38), a push carries another subject's token (F-53), and a push leaves after erasure (F-54) |
| MembershipCache | a Billing member passes a Member check, and a removed member stays cached after the removal event |
| SuiteLink, SuiteInbox | no `_current` config, only the fixed design |

Each model checks small constants, such as three tabs or two replicas. A model is an abstraction,
and it can drift from the code it describes. Two known residuals are reproduced but not run in
CI: `BffSessionFamily/residual/LostResponse.cfg`, where a response lost after commit still logs
the browser out, and `MembershipCache/residual/NaiveEviction.cfg`. See
[`models/README.md`](../../models/README.md).

### Kani harnesses

Kani proves pure kernels for every input within the bounds each harness states in its `Bounds:`
line. `make proofs-list` prints the table, and the `kani` step fails on a harness without bounds.
Outside those bounds nothing is proven.

### Verus

V-01 to V-03 prove the HLC step, the token bucket and the pull page merge for traces of any length.
Verus runs on twins in `rust/verus`, not on the production code. Proptests and Kani harnesses in
`rust/verus/equivalence` tie each twin to production, and a twin can still drift in ways they miss.

### Cross-language vectors

`fixtures/**/generated-v1.json` come from the generators in `scripts/vector-generators.toml`. The
`vectors` step fails when the committed files differ from the generator output. Vectors show that
implementations agree on the listed cases only.

### Known gaps

These come from the known gaps in [verification targets](../verification.md#known-gaps):

- K-12 runs at reduced width: `now` is an `i16` and `duration` a `u8 + 1`. A proptest samples the
  full `i64` range instead.
- The pull page merge has no Kani equivalence harness, because CBMC runs out of memory on the
  `Uuid` compare and sort. Its twin link is a 4096-case proptest. Verus proves V-03 itself.
- K-18 proves the OAuth state length gate only. The `oauth_state_parse` fuzz target covers the
  decoder.
- K-21 proves per byte, over every 3-byte string and every single `char`. Proptests cover longer
  segments.
- No old binary runs against the new schema. The `[old_binary] tests` list in
  `rust/migrations.toml` is empty.
- Suite receivers still accept live deliveries signed only with v1.
- Nothing checks that a new kernel, protocol or decoder gets the tool its class calls for. Review
  catches that, or nobody does.

## Reference

- [Baukit threat model](../threat-model.md)
- [Verification targets](../verification.md)
- [`models/README.md`](../../models/README.md)
- [`baukit-test` README](../../rust/crates/baukit-test/README.md)
- [`baukit-erasure` README](../../rust/crates/baukit-erasure/README.md)
- [`lint/README.md`](../../lint/README.md)
- [ADR 0011: separate API and ops listeners](../adr/0011-separate-api-and-ops-listeners.md)
- [ADR 0013: code rules enforced by baukit check](../adr/0013-code-rules-enforced-by-baukit-check.md)
- [ADR 0015: request admission](../adr/0015-request-admission.md)
- [ADR 0016: outbound HTTP through egress](../adr/0016-outbound-http-through-egress.md)
- [ADR 0017: identity providers and bearer verification](../adr/0017-identity-providers-and-bearer-verification.md)
- [ADR 0018: credential vault records](../adr/0018-credential-vault-records.md)
- [ADR 0027: verification strategy](../adr/0027-verification-strategy.md)
- [Migration: security suites and composed API](../migrations/security-suites-and-composed-api.md)
- `templates/backend/backend/tests/conformance.rs`
- `templates/backend/backend/tests/composed/`
- `templates/backend/backend/fuzz/`
- `templates/common/scripts/repo-lint.sh`
- `agent-skills/skills/baukit-threat-model/SKILL.md`
- `agent-skills/skills/baukit-security-testing/SKILL.md`
