# ADR 0033: The backend signs short-lived media grants in Rust and nginx verifies them in njs, both held to one vector file

## Status

Accepted, 2026-11-27.

## Context

Products serve private media, such as recordings, from object storage or a
volume behind nginx. Streaming every byte through the API process costs a
connection per playback and puts the API on the media path. A plain static
URL leaks to anyone who sees it.

The edge cannot call the backend per request, so it has to check a grant on
its own. That puts the same signature format in two languages, Rust in the
backend and JavaScript in nginx's njs. Two hand-written implementations drift
unless one set of cases binds both.

## Decision

A media grant is the query
`expires=<unix seconds>&keyId=<id>&mode=playback&signature=<base64url>` on a
media path. The signature is HMAC-SHA256 over the string from
`signing_input(path, expires, key_id)`. The exact rules are in
[`deploy/media-grants/README.md`](../../deploy/media-grants/README.md).

- The signer and the Rust verifier live in `baukit_core::media_grant`, behind
  the `media-grants` feature. `MediaGrantKey::from_base64url` loads a key of
  at least `MIN_SECRET_BYTES` (32). `MediaGrantKeyRing` holds the current and
  an optional previous key. `MediaGrantKeyRing::sign` returns a `MediaGrant`
  whose `query()` the backend appends to the path. `MediaGrantKeyRing::verify`
  returns a `VerifiedMediaGrant`. A grant lives at most
  `MAX_GRANT_LIFETIME_SECONDS` (3600), and verification allows
  `MAX_CLOCK_SKEW_SECONDS` (60). `valid_media_path` limits paths to
  `MAX_PATH_BYTES` of plain segments.
- The edge verifier is `deploy/media-grants/njs/media-grant.js`. It reads its
  keys from `MEDIA_GRANT_KEY_ID`, `MEDIA_GRANT_SIGNING_KEY` and the two
  `MEDIA_GRANT_PREVIOUS_*` variables, so no nginx variable holds a secret. It
  refuses every request with 403 when the keys are invalid, and it rejects
  encoded paths, extra query fields and absolute-form request targets.
- The product decides which paths a user may play and signs a grant per
  request. nginx `location` blocks hold the path allowlist.
- Rotation moves the current key to previous, so grants signed with either key
  verify until the old one is dropped.

## Enforced by

- `fixtures/media-grants/vectors-v1.json` holds protocol constants, key and
  ring cases, independently computed signatures and verify cases with their
  expected error codes. The file is hand-written, not generated, so the
  `vectors` step does not cover it.
- Rust tests in `rust/crates/baukit-core/src/media_grant.rs` read that file:
  `protocol_constants_match_the_vectors`, `error_codes_match_the_vectors`,
  `sign_cases_produce_the_independent_signatures` and
  `verify_cases_accept_or_reject_with_the_expected_code`. They run in the
  `rust-tests` step.
- The `media-grants` gate step runs `make media-grants-test`, which runs
  `deploy/media-grants/njs/media-grant.test.mjs` under Node against the same
  file, and `make media-grants-njs-test`, which runs
  `run-njs-vectors.js` in the pinned nginx njs image under both the njs and
  QuickJS engines. The second needs Docker.
- The `kani` step checks the harnesses in `media_grant.rs`, such as
  `check_expiry_bounds`, with the `media-grants` feature.

## Consequences

Media bytes go from storage to the client through nginx without touching the
API. A grant cannot be revoked before it expires, so lifetimes stay short. Any
change to the format has to land in Rust, njs and the vector file together,
and nginx has to run with njs and the key variables set.

## History

- 2026-09-28, `acfe19e0`: signed media grants added to `baukit-core`.
- 2026-09-28, `5d991558`: the njs verifier added under `deploy/media-grants`.
