# Changelog

All notable changes to `baukit-openapi` are documented here.

## [Unreleased]

### Added

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
