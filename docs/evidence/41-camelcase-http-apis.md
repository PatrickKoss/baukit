# 41. camelCase HTTP APIs

Plan item 5. Baukit, its templates, and its clients switch every HTTP and event
wire name to camelCase in one break, and `baukit-openapi` gains the check that
keeps it that way. Steps 1 to 3 of the item land here. The product migration
and the data reset (steps 4 and 5) are follow-ups listed at the end.

## Source revisions

Baukit baseline is `b430d27`. Product schemas were read on 2026-09-27 at the
revisions below. Eigenruhe and Schlauzug have moved since the survey; the
counts are from the revision listed, not from their current HEAD.

The naming check itself produced these counts on each committed
`backend/openapi.json`. "Violations" counts every occurrence with its JSON
pointer, so a name used in five schemas counts five times.

| Product              | Revision  | Violations | Distinct properties | Distinct parameters | Standard-defined names |
| -------------------- | --------- | ---------- | ------------------- | ------------------- | ---------------------- |
| Eigenruhe            | `f74cebb` | 342        | 185                 | 5                   | none                   |
| Hebkit               | `841bf5d` | 755        | 279                 | 16                  | none                   |
| Leitbild             | `bd38b33` | 78         | 40                  | 5                   | none                   |
| Redemut              | `a782538` | 137        | 75                  | 2                   | none                   |
| Runtime Analyzer     | `d47bfd5` | 817        | 209                 | 4                   | none                   |
| Schlauzug            | `31d3f55` | 318        | 175                 | 2                   | none                   |
| Solo Leveling System | `3461eaf` | 1004       | 382                 | 46                  | `access_token`, `expires_in`, `token_type` |
| Tiefgang             | `2d37a06` | 202        | 116                 | 4                   | none                   |

The distinct counts match the plan's survey within three names. Eigenruhe,
Schlauzug, and Solo Leveling System gained a few fields since then. Runtime Analyzer's
`expires_in_days` is a product field, not an OAuth name, so it gets renamed and
not exempted. Solo Leveling System is the only product that needs an exemption
list.

The test fixture `rust/crates/baukit-openapi/tests/fixtures/snake-case-excerpt.json`
is a trimmed copy of Leitbild's `backend/openapi.json` at `bd38b33`: two paths
(`GET /v1/program-runs` and the section-response `PUT`), their 200 and 400
responses, and the eight schemas they reference. The title and version were
replaced so the fixture carries no product name. It holds 16 violations,
including two query and two path parameters.

## Baukit owner and public types

`baukit-openapi` owns the rule:

- `is_camel_case(name)`: an ASCII lowercase letter followed by ASCII letters
  and digits. `userID` passes; `request_id`, `RequestId`, and `_id` fail.
- `find_naming_violations(&Value, exemptions) -> Vec<NamingViolation>` walks a
  serialized document. It checks every key of every `properties` object
  (request and response bodies, nested objects, multipart form fields, and
  `additionalProperties` value schemas) and the `name` of every parameter
  object whose `in` is `path` or `query`. It skips `enum`, `const`, `default`,
  `example`, `examples`, `discriminator`, and `x-` keys, and header and cookie
  parameters. `additionalProperties` map keys are data and never appear in a
  document, so they need no special case.
- `NamingViolation { pointer, name, kind }` with `NameKind::{Property, Parameter}`.
  The pointer escapes `~` and `/` per RFC 6901.
- `check_camel_case_names(&OpenApi, exemptions)` returns
  `SchemaError` with the new `SchemaErrorKind::Naming`, and
  `assert_camel_case_names` panics with every violation listed.
  `baukit-test` re-exports both as `check_openapi_camel_case` and
  `assert_openapi_camel_case`.

Exemptions are caller-supplied names accepted wherever they occur. Baukit ships
no default list because only one product needs one.

### Renamed wire fields

| Owner | Type or site | Before | After |
| --- | --- | --- | --- |
| `baukit-openapi`, re-exported by `baukit-http` | `ErrorBody` | `request_id` | `requestId` |
| `baukit-core` | `pagination::Page` | `next_cursor` | `nextCursor` |
| `baukit-http` | `RequestLocale` rejection detail | `accept_language` | `acceptLanguage` |
| `baukit-ratelimit` | rejection detail | `retry_after` | `retryAfter` |
| `baukit-auth` | `ApiTokenPolicyRejection` detail names | snake_case accepted | camelCase required |
| `baukit-integrations` | `ConnectionStatus` | `last_success_at`, `last_attempt_at`, `last_error_code`, `next_retry_at`, `failed_attempts` | `lastSuccessAt`, `lastAttemptAt`, `lastErrorCode`, `nextRetryAt`, `failedAttempts` |
| `baukit-events`, `@baukit/events` | `EventEnvelope` | `event_id`, `user_id`, `occurred_at`, `source_app`, `schema_version` | `eventId`, `userId`, `occurredAt`, `sourceApp`, `schemaVersion` |
| `baukit-events`, `@baukit/events` | `IngestOutcome` | `ledger_entry_id` | `ledgerEntryId` |
| `@baukit/events` | `EventPayloadSchema` keys | `/^[a-z][a-z0-9_]{0,63}$/` | `/^[a-z][a-zA-Z0-9]{0,63}$/` |
| `@baukit/api-runtime` | `ApiErrorEnvelope`, `parseApiErrorEnvelope` | reads `request_id` | reads only `requestId` |
| offline-readiness contract | `resync_required` detail, pull page | `horizon_revision`, `has_more` | `horizonRevision`, `hasMore` |

`@baukit/sync-client` loses `toSnakeCaseSnapshot`, `toSnakeCaseFailure`,
`SnakeCaseSyncStatusSnapshot`, and `SnakeCaseSyncFailure`. They projected the
camelCase status snapshot onto snake_case names for screens that mirrored a
snake_case API. With camelCase APIs they only keep the old names alive, and
the compatibility stance rules out aliases.

The payload key rule in `@baukit/events` is a decision, not a survey finding.
The envelope fixes payload keys as a lower-case identifier set; leaving them
snake_case would put the only snake_case keys left in a suite event inside its
payload. The Rust side does not validate payload keys, before or after.

Types left alone, each for a stated reason:

- `baukit-ops` responses (`duration_ms`, `accepting_traffic`, `service_name`,
  `rust_version`): the convention excludes operational endpoints.
- `baukit-config`, `baukit-push` `PushConfig`, and `limits.json`: configuration
  files, not wire types.
- `baukit_jobs::Job` and `NewJob`, and job payloads such as the template's
  `ItemCreatedJob`: persistence, which the convention says not to rename.
- Expo push requests, OIDC discovery and token fields in `baukit-auth`,
  `baukit-test`'s mock OIDC server, and `@baukit/auth-*`: formats a third party
  defines.
- `FakeWebhookBody` in `baukit-test`: a stand-in for a provider's webhook body.
- Analytics event properties such as `schema_version` and `app_version`: these
  are PostHog properties under the analytics privacy contract, not HTTP API
  fields. Renaming them is a separate decision with dashboard impact.
- Log fields such as `request_id` in the telemetry spec: log schema, not wire.
- Values: error codes, `ConnectionHealth` and `IngestOutcomeStatus` values,
  event type names, and `operationId` values.
- The shared event fixture's own harness keys (`contract_version`,
  `expected_code`, `expected_user_id`, the `constants` block): test metadata,
  not envelope fields.

`ResponseEnvelope` (`data`, `meta`), `PageParams` (`limit`, `cursor`), and the
`baukit-sync` clock types were already compliant.

### Rejection details do not carry field names

The plan says JSON rejection details take their field names from serde. They do
not. The classified `ApiJson` rejection reports `{"body": "must match the
request schema"}` or `{"body": "must contain valid JSON"}` and never names the
failing field, and `ApiQuery` reports `{"query": ...}`. That was a deliberate
privacy choice in item 2: serde messages quote input and field names. So there
is no field name to rename.

The new test `camel_case_bodies_reject_snake_case_names_without_echoing_them`
in `baukit-http` proves the useful half instead: a `rename_all = "camelCase"`
DTO with `deny_unknown_fields` accepts `startedAt`, rejects `started_at` with
422 `validation_failed` and `{"body": ...}`, and the response never contains
`started_at`. Field-level details that products build by hand with
`ApiError::validation_field` must use the wire name; the add-endpoint skill now
says so and its example uses `startsAt`.

## Supported runtimes

Unchanged: the Rust MSRV, Node from `typescript/.nvmrc`, React Native through
the existing packages. The naming check is pure Rust over `serde_json::Value`
and adds no dependency.

## Failure behavior

- The check never panics except in the `assert_` form. It reports every
  violation in one run, in the document's key order, instead of stopping at
  the first.
- A document that fails to serialize returns `SchemaError` with the serde
  cause, as the drift check does.
- An old client that reads `request_id` gets `undefined`. `@baukit/api-runtime`
  treats an envelope without `requestId` as not a Baukit error and raises
  `HttpError` with the status, so a mismatched server shows up as an
  unexpected response rather than a silent empty request ID.
- `ConnectionStatus` JSON stored by an older release no longer deserializes
  because `failedAttempts` is missing. Its `Option` fields would silently read
  as `None`. Products that store it as JSON reset that column as part of step 5.

## Privacy boundary

Nothing new crosses it. The check reads a schema document, which holds no user
data. The rejection test confirms that a rejected body's field names stay out
of the response.

## Templates

- `__app__-api` DTOs (`ItemDto`, `SaveItemRequest`, `CurrentUserDto`) carry
  `#[serde(rename_all = "camelCase")]`. Their fields were single words, so the
  schema only changes through `requestId`.
- Both committed template schemas, both `generated/openapi.d.ts` files, the MCP
  package's copied `schema.d.ts`, and the web auth test use `requestId`.
- `backend/tests/openapi_drift.rs` gains `openapi_names_are_camel_case` with an
  empty `STANDARD_DEFINED_NAMES` list. The generated CI already runs this file.
  The strict `scripts/quality-gate.sh` now runs it explicitly after the schema
  diff.
- `docs/openapi-drift.md`, the strict profile doc, and the add-endpoint skill
  describe the rule and the exemption list.

The golden trees changed only in the files above plus the generated
`CHANGELOG.md`.

## Breaks

Every row of the renamed-field table is a break with no alias, as the
compatibility stance requires. The crate changelogs and the changeset
`typescript/.changeset/camelcase-wire-names.md` name each field. ADR 0003 has an
amendment explaining why the envelope stays version 1.

## Product adoption

Each product migrates in one change covering the backend and every client, and
turns on the naming check in its `openapi_drift` test in the same change.

- Leitbild first: 40 properties and 5 parameters. Rename the API DTOs and
  query structs, regenerate the schema and clients, enable the check, and
  reset local and development data.
- Eigenruhe: rename 185 properties and 5 parameters; switch the suite event
  sender and receiver to the camelCase envelope; drop `toSnakeCaseSnapshot` in
  `mobile/src/sync/store.ts`; read `retryAfter` in `mobile/src/api.ts` and
  `mobile/src/sync/transport.ts`; send `horizonRevision` in `resync_required`.
- Hebkit: rename 279 properties and 16 parameters; switch
  `hebkit-services/src/suite_events.rs` and the worker's `suite_events.rs`
  adapter to the renamed envelope; drop `toSnakeCaseSnapshot` in
  `mobile/src/sync/store.ts`; update `retry_after` readers in the nutrition
  catalog client and sync transport.
- Tiefgang: rename 116 properties and 4 parameters; switch
  `tiefgang-domain/src/events.rs` and `mobile/src/integrations/health/workouts.ts`
  to the renamed envelope; update `mobile/src/sync/transport.ts` and `types.ts`.
- Redemut: rename 75 properties and 2 parameters; its 64 camelCase names
  already comply.
- Runtime Analyzer: rename 209 properties, including `expires_in_days`, and 4
  parameters.
- Schlauzug: rename 175 properties and 2 parameters once its working tree is
  committed.
- Solo Leveling System: rename 382 properties and 46 parameters; exempt
  `access_token`, `expires_in`, and `token_type`.

Products that persist `baukit_integrations::ConnectionStatus` (Eigenruhe,
Hebkit, Tiefgang, Solo Leveling System) check whether they store it as JSON or
as columns; JSON storage needs the reset.

## Tests

- `rust/crates/baukit-openapi/tests/naming.rs`: the camelCase rule, pointers for
  properties and path, query, component, and multipart names, skipped keywords
  and parameter locations, `additionalProperties` value schemas, exemptions,
  both template schemas, the product excerpt, a generated utoipa document, and
  the panic message.
- Serialization tests for `ErrorBody`, `Page`, `ConnectionStatus`, the event
  fixture corpus in Rust and TypeScript, the locale and rate-limit details, and
  the API token detail-name validator.
- `@baukit/api-runtime` accepts `requestId` and rejects a `request_id`-only
  envelope. `@baukit/events` rejects snake_case envelope fields, payload keys,
  and `ledger_entry_id`.
- `examples/minimal-api` gains an `openapi_names_are_camel_case` test.
