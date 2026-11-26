# ADR 0012: HTTP APIs share one error envelope, camelCase names, committed OpenAPI and idempotency keys

## Status
Accepted, 2026-11-27.

## Context

Web, native and MCP clients call many product APIs. If each API shaped errors and names its own
way, every client would need per-API parsing, and a generated TypeScript client would differ from
product to product. Mobile clients also retry writes on flaky networks, so a retried POST must not
apply twice.

## Decision

- Errors use `ErrorEnvelope` from `baukit-openapi`, re-exported by `baukit-http`:
  `{ "error": { "code", "message", "requestId", "details" } }`. `ApiError` carries a snake_case code.
  Its cause is logged and never serialized. `requestId` is the `X-Request-Id` value
  (`X_REQUEST_ID`).
- Success bodies use `ResponseEnvelope<T, M>` with `data` and `meta`.
- JSON field names are lowerCamelCase. Codes and enum values stay snake_case.
- Each product commits its OpenAPI document. The template test `backend/tests/openapi_drift.rs`
  fails when the generated document differs from the committed one, through
  `baukit_openapi::check_no_drift` and `assert_no_drift`.
- Replay-safe writes take an `Idempotency-Key` header (`IDEMPOTENCY_KEY`) of at most 255 bytes.
  `IdempotencyKeyRule` makes it required or optional per route. Errors are
  `idempotency_key_required`, `invalid_idempotency_key`, `idempotency_key_reused` for the same key
  with a different body, and `idempotency_key_in_progress`. See
  [replay-safe mutations](../platform/replay-safe-mutations.md).

## Enforced by

- `rust/crates/baukit-http/src/tests.rs`: `error_envelope_has_the_exact_shared_shape` and
  `timeout_becomes_standard_envelope`.
- `rust/crates/baukit-openapi/tests/naming.rs`: `camel_case_accepts_lower_camel_names_only`.
- `rust/crates/baukit-openapi/src/lib.rs` tests: `drift_check_accepts_matching_schema_and_rejects_changes_in_both_directions`
  and `missing_committed_schema_is_drift`.
- The template's `openapi_drift.rs`: `committed_openapi_has_no_drift` and
  `openapi_names_are_camel_case`, run by the generated fixture gate.
- `baukit_test::check_replay_safe_mutation_conformance` for product routes, and the tests in
  `rust/crates/baukit-http/src/idempotency/tests.rs`.

## Consequences

One client parser handles every product's errors, and generated clients look the same everywhere.
An API change shows up as a committed schema diff in review. Products cannot pick their own naming
style, and each write route needs a decision about idempotency.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): the envelope with snake_case field names.
- 2026-09-27, commit `47931219`: field names moved to camelCase.
