# Changelog

All notable changes to `baukit-openapi` are documented here.

## [Unreleased]

### Changed

- Breaking: moved to utoipa 6. Every public function that takes or returns a utoipa type
  (`OpenApi`, `Operation`, `Header`, `Parameter`) now uses utoipa 6 types, so products must
  upgrade utoipa to 6 with this release. utoipa 6 stores operation parameters and response
  headers as `RefOr<_>`; `document_if_match`, `document_etag`, and header rules insert inline
  values and skip `$ref` entries when they look for an existing `If-Match` or
  `Idempotency-Key` parameter. The generated documents are unchanged.

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Added

- `ErrorResponseRules` documents error responses and response headers from product rules. A
  status rule pairs an `OperationCondition` (`Always`, `Secured`, `HasRequestBody`,
  `HasPathParameter`, `UnsafeMethod`, `HasIdempotencyKey`) with a status and a description, and
  adds a response with the shared error envelope where the operation lacks that status. A header
  rule adds a header to the responses a `ResponseSelector` picks. `standard_headers()` adds
  `X-Request-Id`, `Retry-After` on 429, and `WWW-Authenticate` on 401. `apply` covers every
  operation and `apply_where` takes a path and method filter. `REQUEST_ID_HEADER`,
  `RETRY_AFTER_HEADER`, `WWW_AUTHENTICATE_HEADER`, `request_id_header`, `retry_after_header`, and
  `www_authenticate_header` are public.
- `document_if_match` documents a revision precondition on an operation: a required or optional
  `If-Match` header parameter, plus 400, 412, and, when required, 428 error responses with the
  shared envelope. `document_etag` adds the `ETag` header to inline 2xx responses.
  `if_match_parameter`, `etag_header`, `IfMatchRequirement`, `IF_MATCH_HEADER`, `ETAG_HEADER`,
  `PRECONDITION_REQUIRED_CODE`, `PRECONDITION_FAILED_CODE`, and `INVALID_IF_MATCH_CODE` are public.
- Add a camelCase naming check. `check_camel_case_names(&openapi, exemptions)` returns
  `SchemaError` with one `NamingViolation` (JSON pointer, name, `NameKind`) per property or
  path or query parameter name that is not lower camelCase, and `assert_camel_case_names`
  panics with the list. `find_naming_violations` runs the same walk on a `serde_json::Value`,
  and `is_camel_case` exposes the rule. The walk skips enum and const values, defaults,
  examples, discriminator mappings, `x-` extensions, and header and cookie parameters. The
  exemption list is for standard-defined names such as OAuth 2.0 `access_token`.

### Changed

- Break: `ErrorBody` serializes `request_id` as `requestId`. The documented error envelope is
  now `{ "error": { "code", "message", "requestId", "details" } }`.
- Break: `SchemaErrorKind` gains the `Naming` variant. Exhaustive matches must handle it.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-openapi`.
