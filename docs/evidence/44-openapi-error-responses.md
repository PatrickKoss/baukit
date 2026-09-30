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

## Follow-up 0.5.2 (2026-09-30), mobile PWA

### Product evidence

Tiefgang's item 8 adoption skipped setting `capabilities.pwa = true`, because `baukit doctor`
rejected `pwa` without `web` (`cli/src/lib.rs:1293-1295`) and checked the worker build only under
`web/` (`:1429-1431`). Tiefgang at `7f0fd02` is mobile-only (`web = false`) and still ships a
PWA:

- `mobile/app.config.ts:31` exports the Expo app to static web output, and
  `e2e/Dockerfile.web` serves `mobile/dist` behind nginx for its web e2e suite.
- `mobile/package.json` has `build:sw`, `build:sw:check`, and `@baukit/pwa-web`, and
  `mobile/scripts/build-sw.mjs` copies `@baukit/pwa-web/worker` to
  `mobile/public/baukit-pwa-worker.js`.
- `mobile/public/sw.js` loads it with `importScripts('/baukit-pwa-worker.js')`, and
  `mobile/src/pwa/register-service-worker.ts` registers `/sw.js`.
- Its own `scripts/quality-gate.sh:91` and `.github/workflows/ci.yml:166` already run
  `pnpm build:sw:check` in `mobile/`.

Eigenruhe at `e8d0e05` has the same layout (`web = false`, `pwa = false`,
`mobile/scripts/build-sw.mjs` copying `@baukit/pwa-web/worker`). Evidence note 12 tested the
artifact against that Expo layout and left "Expo PWA" as a separate CLI decision.

### Decision

The plan step is right; the CLI rule was too narrow. `capabilities.pwa` means the product serves
the `@baukit/pwa-web` worker from a web build and the strict gate checks the copied artifact for
drift. An Expo web export is such a build: Expo copies `public/` into the export as Vite does for
the web app.

`baukit doctor` now requires the web or the mobile capability for `pwa`. The worker build is
checked in the app that serves the PWA: `web/` when the product has a web app, otherwise
`mobile/`. The checks are the same in both places: `build:sw` and `build:sw:check` scripts, the
`@baukit/pwa-web` dependency, and `scripts/build-sw.mjs` referencing `@baukit/pwa-web/worker`.
A product with both apps keeps the worker in `web/`, which matches Hebkit (`web = true`,
`pwa = true`). The strict `scripts/quality-gate.sh` of a mobile-only product calls
`pnpm --dir mobile run build:sw:check` when `pwa` is true; products with a web app keep the web
call. The mobile template does not ship a worker script: generated products keep `pwa = false`,
and the mobile README describes the four pieces to add.

Run against a copy of Tiefgang with `pwa = true`, the 0.5.1 CLI reports "the PWA capability
requires the web capability"; the new CLI reports no PWA problem. Its three remaining findings,
`Makefile`, `compose.yaml`, and `mcp/src/cli.ts` not using port offset 200, predate this change.

### Template change

- `templates/common/__strict__/scripts/quality-gate.sh`: the mobile block runs the mobile
  `build:sw:check` when the product has no web app and `capabilities.pwa` is true.
- `templates/common/CLAUDE.md`: mobile-only products get the matching instruction.
- `templates/mobile/mobile/README.md`: new "Optional PWA worker" section.
- `docs/platform/strict-quality-profile.md` names the app that must provide `build:sw:check`.

### Breaks

None for products. The doctor message for a PWA without an app changed to "the PWA capability
requires the web or mobile capability", and PWA messages name the app directory.

### Gates

- CLI `cargo fmt --check`, `clippy --all-targets -D warnings`, and `cargo test` (42 generator
  tests, 12 unit tests). New tests cover the app-directory choice, the mobile worker checks, a
  generated mobile-only product that fails doctor until the worker build is added, and a
  backend-only product with `pwa = true`. The strict generation test asserts which app's
  `build:sw:check` each flavor's runner calls.
- Snapshots re-blessed: `mobile.tree` (`CLAUDE.md`, `AGENTS.md`, `mobile/README.md`),
  `combined.tree` and `strict.tree` (`mobile/README.md` only).
- Generated fixture `--backend --mobile --web`: backend fmt, clippy, tests, and `openapi_drift`;
  web install, build, lint, test, and `test:coverage`; mobile install, `tsc --noEmit`, lint, and
  `test:coverage`: pass. A strict mobile-only product renders a runner that passes `sh -n`.

### Product adoption

- Tiefgang: set `pwa = true` in `baukit.toml`. `mobile/` already has everything doctor checks.
- Eigenruhe: may set `pwa = true` in `baukit.toml` the same way; its `mobile/` build already
  qualifies.

## Follow-up 0.5.2 (2026-09-30), request validation

### Product evidence

Schlauzug adopted `assert_response_matches_openapi` in 0.5.1 (`01124c8`) but kept
`backend/tests/support/schema.rs`, an 18-line `validate_request(document, path, method, body)`.
It reads `paths[path][method].requestBody.content["application/json"].schema`, wraps it with the
document's `components` under the 2020-12 dialect, and asserts no errors. Two tests call it,
`backend/tests/api_schema.rs:97` and `backend/tests/api_rooms.rs:573`. It is the same wrapper the
response check uses, so the gap was a missing entry point and not a missing mechanism.

### Decision

`baukit-test` adds `ObservedRequest`, `check_request_matches_openapi`, and
`assert_request_matches_openapi` in the existing module. The request check shares the body rules
with the response check through one private function, so media type matching, JSON parsing,
format validation, and violation lists behave the same in both directions.

- The input is the raw body and `Content-Type`, the same shape as `ObservedResponse`. A test
  usually has those bytes because it builds the HTTP request from them. Taking a `serde_json::Value`
  would skip the media type check and fork the API.
- A `requestBody` `$ref` resolves through `components.requestBodies`. The hop limit counts both
  `components.responses` and `components.requestBodies`, so a loop still ends.
- An empty body is missing only when the request body is `required`, as OpenAPI defines it. An
  operation without a `requestBody` rejects any body.
- Parameters and headers stay unchecked. No product asked for them.
- `format` is validated, as for responses. The Schlauzug helper did not enable it, so a malformed
  UUID in a request fixture now fails. That is the intended tightening.

`OpenApiResponseError` became `OpenApiContractError`, because the variants are the same for both
directions and a second, identical enum would only add conversions. No product names the error
type: Schlauzug and Redemut (`redemut-api/src/lib.rs`) use only `ObservedResponse` and
`assert_response_matches_openapi`.

### Breaks

- `baukit-test`: `OpenApiResponseError` is renamed to `OpenApiContractError`.

### Adoption

- Schlauzug: delete `backend/tests/support/schema.rs` and its `mod schema` lines in
  `backend/tests/api_schema.rs` and `backend/tests/api_rooms.rs`. At both call sites serialize
  the body with `serde_json::to_vec` and call `assert_request_matches_openapi` with
  `ObservedRequest { method: "POST", path, content_type: Some("application/json"), body }`, or
  pass the bytes the test already sends.

### Gates

- `cargo test --manifest-path rust/Cargo.toml -p baukit-test --all-features -- --include-ignored`:
  pass.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  with the rust manifest: pass.
