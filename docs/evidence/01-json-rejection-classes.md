# JSON rejection classes evidence

## Source product files

- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-api/src/json.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-api/src/lib.rs`
- `/home/patrick/projects/eigenruhe/backend/tests/hardening.rs`
- `/home/patrick/projects/eigenruhe/docs/quotas-and-rate-limits.md`

## Observed failure or repeated glue

Eigenruhe has a local `ApiJson<T>` extractor because Baukit's extractor turns every Axum JSON
rejection into one 400 response. Without the local extractor, a route body limit loses the public
413 `payload_too_large` response required by Eigenruhe's clients.

## Baukit owner

`baukit-http` owns JSON extraction and the standard error envelope.

## Public types and errors

`JsonRejectionCodes` holds stable codes for body-too-large, content-type, syntax, and data-shape
failures. `HttpOptions::with_json_rejection_codes` enables classified responses. Existing
`with_json_rejection_code` remains the single-code compatibility setting. Invalid codes return
`HttpOptionsError::InvalidJsonRejectionCode`.

## Product-owned inputs

Products own route body limits, chosen stable codes, client recovery behavior, and user-facing text.

## Cases

- Concurrency: classification uses immutable request extensions and adds no shared mutable state.
- Failure: body limits return 413, content-type failures return 415, syntax failures return 400, and
  data-shape failures return 422.
- Privacy: responses and logs omit submitted bodies, Axum rejection text, and serde parser details.
- Cleanup: the extractor allocates no resource that survives the request.

## Supported runtimes

All Rust targets supported by `baukit-http` and Axum 0.8.

## Product adoption change

After Eigenruhe pins the Baukit release, configure `JsonRejectionCodes::new` with
`payload_too_large` for body size and `validation_failed` for the other three classes. Remove
`mod json`, re-export `baukit_http::ApiJson` in `backend/crates/eigenruhe-api/src/lib.rs`, and delete
`backend/crates/eigenruhe-api/src/json.rs`. Existing handler imports remain valid, and
`backend/tests/hardening.rs` keeps its current 413 code assertion.
