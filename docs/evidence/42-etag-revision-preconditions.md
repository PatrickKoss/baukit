# Item 6 evidence: strong-ETag revision preconditions

This note covers item 6 of the [cross-product feature plan](../cross-product-feature-plan.md):
a shared formatter and `If-Match` parser for revision ETags, typed 428, 412, and 400 errors, a
stale-revision helper, `ETag` and `Location` in the default exposed headers, and OpenAPI helpers.
Steps 1 to 3 are done here. Step 4, adoption in Hebkit and Eigenruhe, is deferred.

## Source revisions

Baukit baseline: `7ee710e`. Products were read at Eigenruhe `f74cebb` (the plan cites `e44ff88`),
Hebkit `841bf5d`, Leitbild `bd38b33`, Redemut `a782538`, Runtime Analyzer `d47bfd5`, Schlauzug
`31d3f55` (the plan cites `fb280df`; `schlauzug-api/src/lib.rs` had no uncommitted change),
Solo Leveling System `3461eaf`, and Tiefgang `2d37a06`.

## Source product files

Product paths are relative to `/home/patrick/projects/<product>/backend/crates/`.

- Eigenruhe `eigenruhe-api/src/freshness.rs:4-28`, a second copy in
  `eigenruhe-api/src/settings_handlers.rs:94-117`, and the stale error at `eigenruhe-api/src/lib.rs:748-756`
- Hebkit `hebkit-api/src/adapters/http/preconditions.rs:5-28`, codes and messages in
  `error.rs:125-129` and `:543-545`, and OpenAPI post-processing in `openapi_contract.rs:158-166`
  and `:270`
- Leitbild `leitbild-api/src/journal.rs:667-684` and the stale error at `:844-848`
- Redemut `redemut-api/src/lib.rs:254-272` and the stale error at `:411-413`
- Runtime Analyzer has no `If-Match` parser. It takes `expected_revision` in the JSON body
  (`finops-api/src/routes/budgets.rs:61`, `clusters.rs:53`, `finops-domain/src/patch.rs:34-36`)
  and returns 412 `stale_revision` (`finops-api/src/routes/admin_common.rs:49-51`).
- Schlauzug `schlauzug-api/src/lib.rs:1280-1302`, the stale error at `:1305-1325`, the ETag at
  `:1400`, and the OpenAPI entry at `:5568`
- Solo Leveling System `sl-api/src/v1/me.rs:186-264`
- Tiefgang `tiefgang-api/src/mutation_headers.rs:19-43`, the stale error at `error.rs:313-316`,
  and hand-written OpenAPI in `contract.rs:66-80` and `:124`

## Observed failure or repeated glue

Seven products format and parse revision ETags by hand, and no two parsers agree. Each one is a
`strip_prefix`, `strip_suffix`, and integer parse, so the edge behavior falls out of whichever
integer type the author picked. Eigenruhe and Redemut accept `"+1"`, `"01"`, and `"-0"` through
`i64::from_str`. Leitbild accepts `"j-01"` and caps revisions at 2^53 - 1. Five products read only
the first `If-Match` value and ignore the rest. The 400 code is one of `validation_failed`,
`invalid_precondition`, `invalid_if_match`, or `bad_request`, and the 412 code is one of
`precondition_failed`, `stale_revision`, `revision_mismatch`, `draft_version_conflict`, or
`stale_resource_version`. Every product that sends an ETag also lists `etag`, `location`, and
`if-match` in its CORS configuration by hand.

## Parser disagreement

The vector names refer to `parseCases` in `fixtures/etag-preconditions/vectors-v1.json`. "400" in
a cell means the product rejects the value with its generic malformed-value error, not with a
specific reason.

| Case | Eigenruhe | Hebkit | Leitbild | Redemut | Schlauzug | Solo Leveling System | Tiefgang | Baukit |
|---|---|---|---|---|---|---|---|---|
| Tag format | `"{kind}-N"` | `"rev-N"` | `"j-N"` | `"N"` | `"draft-N"` | `"{uuid}:{micros}"` | `"revision:N"` | `"{prefix}N"` |
| Missing header (`missing-required`, `missing-optional`) | optional | 428 | 428 | optional | 428 | optional | optional | caller chooses |
| `revision-zero` | accepted | 400 | 400 | accepted | accepted | n/a | 400 | accepted |
| `revision-leading-zero`, `revision-plus-sign` | accepted | 400 | accepted | accepted | `01` accepted, `+1` 400 | accepted | accepted | 400 `invalid_revision` |
| `revision-negative-zero` | accepted as 0 | 400 | 400 | accepted as 0 | 400 | accepted | 400 | 400 `invalid_revision` |
| Upper bound (`revision-i64-max`, `revision-above-js-safe-integer`) | i64 | i64 | 2^53 - 1 | i64 | u64 | i64 micros | i64 | i64 |
| `repeated-header` | first value | 400 | first value | first value | 400 | first value | first value | 400 `repeated_header` |
| `list-of-two` | 400 | 400 | 400 | 400 | 400 | 400 | 400 | 400 `list` |
| `wildcard` | 400 | 400 | 400 | 400 | 400 | 400 | 400 | 400 `wildcard` |
| `weak-validator` | 400 | 400 | 400 | 400 | 400 | 400 | 400 | 400 `weak_validator` |
| `surrounding-whitespace` | 400 | 400 | 400 | 400 | 400 | 400 | 400 | accepted |
| `prefix-of-another-resource`, `prefix-case-differs` | 400 | 400 | 400 | 400 | 400 | 412 for another UUID | 400 | 400 `prefix_mismatch` |
| 400 code | `validation_failed` | `invalid_precondition` | `validation_failed` | `bad_request` | `invalid_if_match` | `invalid_if_match` | `validation_failed` | `invalid_if_match` |
| 412 code | `stale_revision` | `precondition_failed` | `precondition_failed` | `precondition_failed` | `draft_version_conflict` | `stale_resource_version` | `revision_mismatch` | `precondition_failed` |
| 412 details | `current_revision` | none | none | none | `current_draft_version`, `updated_at` | none | none | `currentRevision` when known |

Runtime Analyzer is left out of the table because it sends the revision in the request body.

## Baukit owner

`baukit-http` owns the formatter, the parser, the stale helper, and the mapping to `ApiError`.
`baukit-openapi` owns the OpenAPI helpers and the three error-code constants, because
`baukit-http` depends on `baukit-openapi` and not the other way round. The fixture lives in
`fixtures/etag-preconditions/vectors-v1.json` so a future TypeScript client can test against the
same cases.

## Public types and errors

`baukit-http`:

- `Revision`, a counter in `0..=i64::MAX`, with `Revision::MAX`, `get`, `to_i64`, `From<u32>`,
  `TryFrom<u64>`, and `TryFrom<i64>`. `RevisionOutOfRange` is the conversion error and maps to a
  500.
- `RevisionEtag<'a>`, built with `const fn new` for a fixed prefix or `try_new` for a runtime
  prefix. It has `format`, `header_value`, `parse`, `required_if_match`, and `optional_if_match`.
  `InvalidEtagPrefix` is the `try_new` error. `MAX_ETAG_PREFIX_BYTES` is 64.
- `InvalidIfMatch`, one variant per rejection, with a stable `reason()` string.
- `PreconditionError`, with `Required`, `Invalid(InvalidIfMatch)`, and `Stale { current }`, plus
  `status()`, `code()`, and `From<PreconditionError> for ApiError`.
- `ensure_current_revision(expected, current)`.

`baukit-openapi`:

- `IfMatchRequirement`, `if_match_parameter`, `etag_header`, `document_if_match`, and
  `document_etag`.
- `IF_MATCH_HEADER`, `ETAG_HEADER`, `PRECONDITION_REQUIRED_CODE`, `PRECONDITION_FAILED_CODE`, and
  `INVALID_IF_MATCH_CODE`.

## Decisions

### Revision 0 is accepted

Eigenruhe, Redemut, and Schlauzug store revisions that start at 0, and Hebkit, Leitbild, and
Tiefgang start at 1. A parser that rejects 0 would force the first three to renumber rows before
they could adopt it. The formatter takes any `Revision`, and parse must round-trip what format
produces, so `"rev-0"` parses. A product that never issues 0 loses nothing: the stale helper
rejects it with 412 like any other revision the row does not hold. Hebkit clients that send
`"rev-0"` see 412 instead of 400 after adoption.

### Surrounding whitespace is trimmed

RFC 9110 section 5.5 says optional whitespace around a field value is not part of the value.
The parser trims SP and HTAB at both ends and rejects whitespace inside the tag as `malformed`.
Every product parser rejected a padded value today, but a client or proxy that pads the value is
following the RFC, and the server has no reason to punish it.

### The prefix is case-sensitive

RFC 9110 section 8.8.3.2 defines strong comparison as octet equality, and the prefix is part of
the opaque tag. `"REV-42"` is not the tag the server issued, so it fails with `prefix_mismatch`.
All seven parsers already compare the prefix byte for byte.

### Canonical decimal only

The server formats revisions without a sign or leading zeros, so any other spelling was not issued
by the server. Rejecting `+1`, `01`, `00`, and `-0` keeps the tag a strong validator: one revision
has exactly one tag. Hebkit already enforces this with a round-trip check.

### Upper bound is `i64::MAX`

Every product stores the revision in a PostgreSQL `BIGINT`. `u64` would let a client name a
revision the database cannot hold. Leitbild's 2^53 - 1 cap exists for its JSON body copy of the
ETag, which is a product concern, so the vectors include `revision-above-js-safe-integer` as an
accepted case.

### Prefix rules

A prefix is empty or up to 64 bytes of `[A-Za-z0-9._:-]`. The empty prefix covers Redemut. The
colon covers Tiefgang's `revision:` and Solo Leveling System's `{uuid}:`. Quotes, commas, slashes,
and whitespace are excluded so a prefix can never make a tag ambiguous inside an `If-Match` list.
`RevisionEtag::new` is a `const fn` that panics on an invalid prefix, so a `const` tag with a bad
prefix fails to compile. `try_new` returns `InvalidEtagPrefix` for a prefix built at run time.

### Prefix mismatch is 400, not 412

RFC 9110 treats any non-matching `If-Match` as a failed precondition. The products instead treat a
tag for another resource kind as a client bug, and so does Baukit: refetching would not help, and
a 412 would send a client into a reload loop. Solo Leveling System returns 412 for another
resource's UUID today. It can keep that by checking the UUID before calling the parser, or accept
the 400.

### One 400 code with a reason

`invalid_if_match` is the single 400 code, and `details.reason` carries one of `repeated_header`,
`empty`, `non_ascii`, `wildcard`, `weak_validator`, `list`, `malformed`, `prefix_mismatch`,
`invalid_revision`, or `revision_out_of_range`. Clients branch on the code. The reason is for logs
and support. Schlauzug and Solo Leveling System already use the code name.

### Repeated headers and lists are rejected

Two `If-Match` fields combine into a list under RFC 9110, so a repeated header is the same request
as a list. Neither can express "update if the revision is one of these" for a counter, and reading
only the first value lets a proxy decide which precondition applies.

### `currentRevision` in the 412

`Stale { current: Some(_) }` adds `details.currentRevision`, so a client can tell whether its copy
is older or newer than the server's. `ensure_current_revision` always sets it. A product that does
not want to reveal the revision builds `Stale { current: None }` directly. The field name is
camelCase per the brief.

### Default CORS headers

`ETag` and `Location` join the default exposed headers, as the plan asks. `If-Match` also joins
the default allowed request headers. The plan does not name it, but a browser cannot send a
conditional write without it, and every product that parses `If-Match` lists it by hand.

### OpenAPI helpers

`document_if_match` replaces any `If-Match` header parameter, matched without regard to case, and
adds 400, 412, and, for required routes, 428 responses that point at `ErrorEnvelope`. It keeps any
response the operation already documents. Item 8 also adds error responses to operations, so the
two may overlap. `document_etag` adds the `ETag` header to inline 2xx responses and leaves `$ref`
responses alone, because those are shared components.

## Product-owned inputs

Products own the prefix, whether a route requires `If-Match`, where the revision is stored and
incremented, the transaction that compares and writes, and whether a stale error reveals the
current revision. Timestamp versions such as Solo Leveling System's microseconds fit the parser
only as a number; the product keeps the conversion.

## Supported runtimes

All Rust targets supported by `baukit-http` and `baukit-openapi`, Axum 0.8, and utoipa 5, with
Rust 1.95 or newer. The parser takes a `HeaderMap` or `HeaderValue` and needs no async runtime.

## Failure behavior

| Condition | Status | Code | Details |
|---|---|---|---|
| Required header missing | 428 | `precondition_required` | none |
| Header present but not one strong tag for this prefix | 400 | `invalid_if_match` | `reason` |
| Revision is not current | 412 | `precondition_failed` | `currentRevision` when known |
| `Revision::try_from` out of range on the server | 500 | internal error | none |

All four go through `ApiError`, so they carry the request id and CORS headers like any other error.

## Privacy boundary

Errors never echo the submitted header value. `details.reason` is a fixed string from the list
above. `currentRevision` is metadata of a resource the caller is already allowed to write, and a
product can omit it.

## Breaks

- The default exposed headers now include `ETag` and `Location`.
- The default allowed request headers now include `If-Match`.

Baukit ignores a name passed to `with_additional_exposed_headers` that is already a default, so a
product that still passes `etag` or `location` keeps working and can drop them at its own pace.

## Template

The backend template has a natural write route: items `PUT` and `DELETE` in
`templates/backend/backend/crates/__app__-api/src/lib.rs:43`. Its table has no revision column, so
the template is unchanged. Adding a revision column and `If-Match` to that route is a separate
change. The generated backend fixture, with and without `--auth oidc`, passes fmt, clippy, tests,
and `openapi_drift` against this branch.

## Deferred: step 4

Hebkit and Eigenruhe adoption is deferred. Client-observable changes when they adopt:

Hebkit:

- 400 `invalid_precondition` becomes 400 `invalid_if_match` with `details.reason`, and the
  message changes from "If-Match must contain a strong revision ETag" to "If-Match must contain
  one strong ETag for this resource".
- The 428 message changes from "If-Match header is required" to "If-Match is required".
- `"rev-0"` returns 412 instead of 400.
- A padded value such as ` "rev-7"` is accepted instead of rejected.
- A 412 gains `details.currentRevision` if Hebkit uses `ensure_current_revision`.
- `docs/API_ERRORS.md` must drop `invalid_precondition` and add `invalid_if_match`.

Eigenruhe:

- 400 `validation_failed` with an `If-Match` field detail becomes 400 `invalid_if_match` with
  `details.reason`.
- 412 `stale_revision` with `current_revision` becomes 412 `precondition_failed` with
  `currentRevision`. The MCP safe-details allowlist in `mcp/src/tools/result.ts:67` must change
  with it, and `docs/patch-semantics.md:14` must be updated.
- `"practice-+1"`, `"practice-01"`, and `"practice--0"` return 400 instead of being accepted.
- A repeated `If-Match` returns 400 instead of using the first value.
- A padded value is accepted.
- OpenAPI gains 400 and 412 responses on routes that take `If-Match`.
- The second parser in `settings_handlers.rs:94-117` goes away.

## Product adoption change

After each product pins the release containing this item:

- Eigenruhe: replace `freshness.rs` and the `settings_handlers.rs` copy with one `RevisionEtag`
  per kind, map stale writes through `ensure_current_revision`, and drop `etag`, `location`, and
  `if-match` from the CORS lists at `lib.rs:405` and `:429`.
- Hebkit: replace `preconditions.rs` with `RevisionEtag::new("rev-")`, delete
  `InvalidPrecondition`, `PreconditionRequired`, and `PreconditionFailed` from `HttpError`, replace
  the `openapi_contract.rs` If-Match and ETag post-processing with `document_if_match` and
  `document_etag`, and drop the hand-listed CORS names in `mod.rs`.
- Leitbild: replace `parse_if_match` in `journal.rs:667-684` with
  `RevisionEtag::new("j-").required_if_match`, and keep the 2^53 - 1 check only for the JSON body
  copy. `"j-01"` becomes a 400.
- Redemut: replace `if_match_revision` and `revision_etag` in `lib.rs:254-272` with
  `RevisionEtag::new("")`, and move 400 `bad_request` to `invalid_if_match`.
- Runtime Analyzer: no change while revisions travel in the body. If it moves to `If-Match`, use
  the helpers and retire `stale_revision`.
- Schlauzug: replace `parse_draft_if_match` in `lib.rs:1280-1302` with
  `RevisionEtag::new("draft-")`. `"draft-01"` becomes a 400, and `draft_version_conflict` becomes
  `precondition_failed` unless the product keeps its own 412 for the draft timestamp.
- Solo Leveling System: use `RevisionEtag::try_new(&format!("{id}:"))` in `me.rs`, convert the
  parsed number to a timestamp in the product, and decide whether another resource's tag stays 412.
  Negative and zero-padded microseconds become 400.
- Tiefgang: replace `mutation_headers.rs:19-43` with `RevisionEtag::new("revision:")`, move
  `validation_failed` to `invalid_if_match` and `revision_mismatch` to `precondition_failed`, and
  replace the hand-written OpenAPI in `contract.rs` with the helpers.

## Product defects found

- Eigenruhe, Leitbild, Redemut, Solo Leveling System, and Tiefgang read only the first `If-Match`
  value, so a second header is silently ignored.
- Eigenruhe and Redemut accept `+1`, `01`, and `-0`, so several spellings match one revision and
  the tag is not a strong validator in practice. Leitbild accepts `j-01`, Schlauzug accepts
  `draft-01`, and Solo Leveling System accepts signed or padded microseconds.
- Eigenruhe carries two parsers for the same format (`freshness.rs` and `settings_handlers.rs`).
