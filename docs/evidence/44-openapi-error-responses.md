# OpenAPI error responses and compatibility evidence

Item 8 of the [cross-product feature plan](../cross-product-feature-plan.md). Steps 1 to 3 land
here. Step 4, product adoption, is listed at the end and has not started.

## Source revisions

- Runtime Analyzer `d47bfd5`: `backend/crates/finops-api/src/openapi_contract.rs:358` (`document_errors`),
  `:447-489` (status insertion by protection, path parameter, method, and the 412 on revisioned
  writes). `scripts/check-openapi-contract.py` checks route parity and style, not compatibility.
- Hebkit `841bf5d`: `backend/crates/hebkit-api/src/adapters/http/openapi_contract.rs:204-230`
  (status list), `:261-320` (`x-request-id`, `WWW-Authenticate`, `Retry-After`, and
  `Cache-Control` headers). `scripts/check-api-compatibility.py` runs in `.github/workflows/ci.yml`.
- Schlauzug `31d3f55`: the untracked `scripts/check-openapi-compatibility.py` supplied the rule
  list. `backend/tests/support/schema.rs:1-50` validates responses with `jsonschema`.
- Redemut `a782538`: `backend/crates/redemut-api/src/lib.rs:4010-4062` validates live responses,
  including the media type and empty 204 bodies, with `jsonschema` 0.56.
- Solo Leveling System `3461eaf`: `scripts/api-compatibility.py`.
- Tiefgang `2d37a06`, Eigenruhe `f74cebb`, and Leitbild `bd38b33`: the decorator line references
  in the plan. Tiefgang's `contract.rs` is the only one that documents `Idempotency-Key`, which is
  why `HasIdempotencyKey` exists.

## Baukit owners

- `baukit-openapi`: `ErrorResponseRules` in `rust/crates/baukit-openapi/src/error_responses.rs`.
- `scripts/check-openapi-compatibility.py` and its test, run by `make scripts-test` and the
  `openapi-compatibility` CI job.
- `baukit-test`: `assert_response_matches_openapi` in
  `rust/crates/baukit-test/src/openapi_response.rs`.
- Templates: `error_response_rules()` in the generated API crate, and
  `quality.openapi_compatibility` in the strict gate.

## Public API

`baukit-openapi` adds:

- `ErrorResponseRules::new()`, `.status(condition, status, description)`,
  `.header(condition, selector, name, header)`, `.standard_headers()`, `.apply(&mut OpenApi)`,
  and `.apply_where(&mut OpenApi, Fn(&str, &HttpMethod) -> bool)`.
- `OperationCondition`: `Always`, `Secured`, `HasRequestBody`, `HasPathParameter`,
  `UnsafeMethod`, `HasIdempotencyKey`.
- `ResponseSelector`: `Every`, `Status(u16)`.
- `REQUEST_ID_HEADER`, `RETRY_AFTER_HEADER`, `WWW_AUTHENTICATE_HEADER`, `request_id_header()`,
  `retry_after_header()`, and `www_authenticate_header()`.

`document_if_match` now inserts its responses through the same function as the status rules.
Its output is unchanged.

`baukit-test` adds `ObservedResponse`, `OpenApiResponseError`,
`check_response_matches_openapi`, and `assert_response_matches_openapi`, and depends on
`jsonschema` 0.58.1 without default features. `jsonschema` pulls in `borrow-or-share`, which is
MIT-0, so `rust/deny.toml` now allows MIT-0. MIT-0 is MIT without the attribution clause.

The CLI adds `OpenApiCompatibility` (`Off`, `Report`, `Enforce`) and the
`QualityManifest::openapi_compatibility` field. Code that builds `QualityManifest` with a struct
literal must set it. Manifests without the key still parse as `off`.

No break in either crate.

## Decorator decisions

The rules are data, and the product owns every status and description. The crate only decides
which operations match, because that is the part the eight products wrote eight times. Nobody
agreed on descriptions, so none ship as defaults.

- A status rule inserts a response only when the operation does not document that status. A
  handler's own 404 description wins over the rule's.
- A header rule skips `$ref` responses and keeps an existing header of the same name, compared
  without case. Hebkit overwrites instead. Keeping the handler's header is the safer default,
  and no product relied on the overwrite.
- `Secured` reads the operation's `security`, falling back to the document's. The anonymous `{}`
  requirement alone does not count; `[{}, {bearer}]` does, because a bad credential still gets a
  401. Runtime Analyzer checks `!is_empty()` and Hebkit `is_some()`. Both agree with `Secured`
  on their documents, because neither has an empty or anonymous-only list.
- Statuses only some routes return, such as Runtime Analyzer's 412 on revisioned `PATCH`
  routes, use `apply_where` with a path and method filter.
- Call `document_if_match` before the rules, so the precondition 400 keeps its
  `invalid_if_match` description.

### Product comparison

I copied each product at the listed revision into a scratch directory, replaced its local
decorator with rules, regenerated the document, and compared it with the committed one.

- Runtime Analyzer: the status rules plus one `apply_where` for 412 gave a byte-identical
  document.
- Hebkit: status rules, `x-request-id` on every response, `WWW-Authenticate` on 401,
  `Retry-After` on 429, and `Cache-Control` on secured operations, applied before its example
  loop, gave a byte-identical document.

Hebkit spells the header `x-request-id` and gives `Retry-After` `minimum: 0.0`.
`standard_headers()` uses `X-Request-Id` and `minimum: 0`, so Hebkit keeps its own header rules
until it picks one spelling. Neither product used `HasRequestBody` or `HasIdempotencyKey`.

`rust/crates/baukit-openapi/tests/error_responses.rs` checks the rules against trimmed, neutral
fixtures in the shape of both documents. The full neutral documents (116 and 103 paths) also
passed before I trimmed them.

## Compatibility check

The question is: can a client that conformed to the base document fail or lose data against the
current one? Route parity and style belong to other checks and stay in the products.

The 23 rule IDs cover removed operations, parameters, responses, and media types; changed
`operationId` and security; newly required parameters and bodies; new success statuses; and
schema changes. Schema rules know which side of the wire they are on. Requests break when they
narrow and responses break when they widen. Removed properties, changed types, formats, patterns,
and composition break on both sides. The script follows local `$ref`s with a cycle guard,
normalizes path parameter names, merges path-level parameters, and compares header names without
case.

The false-positive policy is to report when unsure. A missed break reaches clients; a false alarm
costs one line in `docs/openapi-accepted-breaks.json`. That is why a response enum that gains a
value counts as a break here, while Hebkit's checker only flags narrowed request enums.

Accepted breaks are `{"accepted": [{"rule", "location", "reason"}]}`. Each field must be a
non-empty string, and the rule must exist. An entry that no longer matches prints a note.

Report-only mode prints to stdout and exits 0. `--enforce` prints to stderr and exits 1 on any
unaccepted break. Bad input exits 2. `--base-revision` reads the file from git and passes with
`skipped` when the revision has no copy yet.

The test suite fails on each rule's break and passes an additive change: a new operation, an
optional parameter, an optional request property, a new response property, and a new error
status.

### Strict profile wiring

`quality.openapi_compatibility` defaults to `off`. `report` and `enforce` need the strict profile
and a backend; `baukit doctor` rejects anything else. The gate runs the script after the drift
test, with the migration guard's base revision. A product generated with `--baukit-path` runs the
script from that checkout. Otherwise the gate clones the template's Baukit tag, so registry
products get the check only after a release that ships the script.

## Response validator decisions

The check takes the documented path template, not the request URL. Matching URLs against
templates adds ambiguity, and both product helpers already pass the template.

- The status matches exactly, then by class (`4XX`), then `default`. Response `$ref`s resolve.
- A body must be present exactly when the response documents content. Redemut allowed empty
  bodies only for 204 and 304; reading the document instead covers both without a list.
- The media type matches exactly, then `type/*`, then `*/*`. Only `application/json` and `+json`
  bodies are validated. Redemut also asserted binary schemas; that stays product-side.
- The schema is wrapped with the document's `components` under the 2020-12 dialect, the same
  trick both products use. Format validation is on, so a malformed UUID fails.
- The error lists every violation with its JSON pointer, not only the first.

## Template changes

The CLI has no changelog, so the template changes are recorded here and in the generated
`CHANGELOG.md` and `README.md`.

- The generated API crate calls `error_response_rules()` after the metadata. It documents 400,
  413, 415, and 422 on JSON bodies, 400 and 404 on path parameters, 500 and 504 everywhere, and
  with auth also 401 on secured operations and 429 everywhere. `standard_headers()` adds the
  headers. The committed `backend/openapi.json`, `generated/openapi.d.ts`, and the MCP
  `schema.d.ts` were regenerated.
- `baukit.toml` gains `quality.openapi_compatibility = "off"`, and the strict
  `scripts/quality-gate.sh` runs the check in `report` or `enforce` mode. The gate now cleans all
  temporary directories from one `EXIT` trap, and the metric-name lint reuses the same Baukit
  checkout.

## Product adoption (step 4, not started)

- Runtime Analyzer: replace the status part of `document_errors` with rules and `apply_where`
  for 412.
- Hebkit: replace the status and header inserts with rules. Keep `x-request-id` as a custom
  header rule or switch to `X-Request-Id` as an intended change. Replace
  `scripts/check-api-compatibility.py` with the Baukit script in `report` mode.
- Solo Leveling System: replace `scripts/api-compatibility.py` with the Baukit script.
- Redemut and Schlauzug: replace the local response validators with
  `assert_response_matches_openapi`.
