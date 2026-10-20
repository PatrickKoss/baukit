# Data subject contract

**Status:** Version 1 contract. Core types, receipt verification, the erasure
server adapter and the controlled client-test target are implemented. Export
and service conformance follow in D3 and T2.
**Applies to:** Services accepting machine-initiated erasure and export requests,
including services implemented without Baukit.
**Related:** [product-profile erasure](./product-profile-erasure-contract.md),
[analytics privacy](./analytics-privacy-contract.md),
[`baukit-data-subject`](../../rust/crates/baukit-data-subject/README.md), and
[receipt fixtures](../../fixtures/data-subject/v1/README.md).

A privacy tool asks a service to find a person by identifiers and erase or export
their data. The service returns an operation resource and a signed receipt.
The receipt contains category counts and an opaque client reference. It contains
no lookup identifiers or record content. The caller verifies the person's
identity before making the request; identity verification is outside this contract.

Implementations outside Baukit follow the paths, JSON fields, authentication,
errors and signing rules below. Baukit products can use the crate's core without
its default features. The core has no Axum or SQLx dependency. The `server`
feature adds the erasure router, PostgreSQL store and durable job handler.
The current server advertises only `operations: ["erasure"]`.

## 1. Discovery

`GET /.well-known/baukit/data-subject` is unauthenticated and returns HTTP 200
with JSON. It is cacheable for five minutes, for example with
`Cache-Control: public, max-age=300`.

```json
{
  "contractVersion": "1",
  "service": { "name": "billing", "version": "4.2.0" },
  "operations": ["erasure", "export"],
  "identifierTypes": ["email", "accountId"],
  "scopes": { "erasure": "data-subject.erase", "export": "data-subject.export" },
  "limits": { "maxIdentifiers": 16, "maxIdentifierLength": 256, "maxExportPartBytes": 8388608 },
  "receiptKeys": [
    {
      "keyId": "<lowercase hex SHA-256 of SPKI DER>",
      "algorithm": "ed25519",
      "publicKeyPem": "<SPKI PEM with PUBLIC KEY label>",
      "status": "current"
    }
  ]
}
```

The placeholders above are explanatory. The fixture discovery document contains
a usable public test key. `operations` lists implemented kinds. `identifierTypes`
lists accepted types. Clients honor the advertised limits; the shown values are
the defaults. All JSON property names are camelCase except DSSE's standard
`keyid` field, which must stay lowercase.

`receiptKeys` has exactly one `current` key. Keys that signed receipts still
retained remain listed as `previous`. Key IDs are unique and must match the
public keys using section 5's rule. Clients obtain discovery over their trusted
service connection and check `contractVersion` before using it.

## 2. Identifiers and normalization

Both POST routes accept this body:

```json
{
  "subjectReference": "client-opaque-reference-01",
  "identifiers": [{ "type": "email", "value": "fixture@example.invalid" }]
}
```

`subjectReference` is an opaque string of 16 to 128 visible ASCII characters,
bytes `0x21` through `0x7e`. Spaces, control characters and non-ASCII text are
invalid. Do not trim or change the reference. A client's HMAC of the subject is
a suitable reference. Never use the person's raw email, account ID or phone.
The operation and receipt echo this reference unchanged.

A request has 1 to `maxIdentifiers` identifiers. Built-in types are `email`,
`accountId`, `customerId` and `phone`. Custom types use the syntax
`x-[a-z0-9]+(-[a-z0-9]+)*`, such as `x-loyalty-id`. Empty suffixes, uppercase,
underscores, repeated hyphens and trailing hyphens are invalid. A syntactically
valid type absent from `identifierTypes` fails with
`data_subject_identifier_type_unsupported`; a malformed type fails with
`data_subject_request_invalid`.

Normalize before matching or comparing requests:

| Type | Rule |
| --- | --- |
| `email` | Trim leading and trailing Unicode whitespace, then apply Unicode lowercase. No additional email address grammar is imposed. |
| `phone` | Require `+` followed by 8 to 15 ASCII digits. Do not trim, insert a country code or remove punctuation. |
| `accountId`, `customerId`, custom | Trim leading and trailing Unicode whitespace, then compare exactly, including case. |

`maxIdentifierLength` counts UTF-8 bytes. Both the raw value and its normalized
value must fit this limit. A normalized value cannot be empty. Preserve the
identifier array's order and duplicates. Changing either changes the normalized
body for idempotency purposes. The byte limit, empty-value rule and custom-type
grammar make the plan's unspecified cases explicit.

Identifiers are transient matching inputs. Never log them, store them in plain
text, place them in a job payload or put them in a receipt. The operation store
keeps only an HMAC of the normalized request for conflict detection. Keep the
HMAC key stable and secret. Neither an unkeyed digest nor a signed receipt is a
safe place for low-entropy lookup values.

The Baukit server resolves normalized identifiers inside the POST transaction.
It queues only `LocalSubject` values, each containing an internal product subject
ID and an optional authentication subject. Products must use opaque internal
IDs, never copy lookup values into these fields. No request identifier is needed
after acceptance, including on retries. Resolution does not erase records.
The product must make erasure safe when a resolved record disappeared meanwhile.
Duplicate local subjects are erased once. Job payloads are removed on success;
terminal failure keeps only the operation ID. A resolver failure occurs before
acceptance and returns a safe HTTP 500 `internal` envelope.

## 3. Operations and idempotency

| Method and path | Required scope | Purpose |
| --- | --- | --- |
| `POST /api/v1/data-subject/erasures` | `data-subject.erase` | Start an erasure. |
| `GET /api/v1/data-subject/erasures/{operationId}` | `data-subject.erase` | Read erasure status and receipt. |
| `POST /api/v1/data-subject/exports` | `data-subject.export` | Start an export. |
| `GET /api/v1/data-subject/exports/{operationId}` | `data-subject.export` | Read export status, parts and receipt. |
| `GET /api/v1/data-subject/exports/{operationId}/parts/{partNumber}` | `data-subject.export` | Download one bounded JSON part. |

POST requires one `Idempotency-Key` header, 16 to 128 visible ASCII characters
with the same byte grammar as `subjectReference`. Missing, repeated or malformed
headers return HTTP 400 `data_subject_idempotency_key_invalid`. Treat the key as
opaque and do not trim it.

A replay with the same key and normalized body returns the same operation.
The body includes the unchanged `subjectReference` and the normalized identifier
array. JSON object field order and insignificant JSON whitespace do not matter.
A different normalized body returns HTTP 409
`data_subject_idempotency_conflict`. Isolate replay records by creating client
and operation kind, just as reads are isolated by client and kind.

POST returns HTTP 202 with a `pending` resource. Pending replay also returns
202. Replaying any terminal operation returns HTTP 200 with that same operation,
including the original receipt or failure. A status GET returns HTTP 200 with
the current resource.

```json
{
  "operationId": "00000000-0000-4000-8000-000000000001",
  "kind": "erasure",
  "status": "pending",
  "subjectReference": "client-opaque-reference-01",
  "createdAt": "2026-10-20T10:00:00Z",
  "completedAt": null,
  "categories": [],
  "restrictions": [],
  "parts": [],
  "receipt": null,
  "error": null
}
```

`operationId` is a UUID. `kind` is `erasure` or `export`. Timestamps are RFC 3339.
Keep all resource fields present, including nulls and empty arrays.

| Status | Meaning | Receipt |
| --- | --- | --- |
| `pending` | Accepted work has not finished. `completedAt` and `error` are null. | Null. |
| `completed` | Requested work finished. | Required. |
| `notFound` | No identifier matched any local subject. | Required, including when nothing was erased or exported. |
| `restricted` | Erasure removed what it could; listed categories remain under a legal obligation under Art. 17(3) GDPR. | Required. |
| `failed` | Accepted work reached terminal failure. `error` contains a safe `code`. | Null. |

All states except `pending` are terminal and have a completion timestamp.
Successful terminal states have `error: null`. A failure's error is
`{ "code": "<safe stable service code>" }`; it must not contain internal causes,
identifiers or provider responses. A failure cannot claim a successful receipt.

`categories` contains `{ "category", "action", "count" }`. Actions are `erased`,
`exported` and `retained`; counts are nonnegative integers. Category names describe
classes of records, never individuals. A restriction contains
`{ "category", "reason", "legalBasis", "retainUntil" }`. Reasons and legal bases
are safe explanations. `retainUntil` is an ISO date `YYYY-MM-DD`, or null if the
end date is unknown. A `restricted` result lists the retained categories and
their obligations. A `notFound` result has empty categories, restrictions and parts.

Clients retain the idempotency key before sending POST. After a timeout or lost
response, retry POST with that same key and body. Once the operation ID is known,
poll its GET route until terminal. Use bounded exponential backoff with jitter,
for example starting at one second and capping at thirty seconds, and honor
`Retry-After` if the service supplies it. These timings are guidance, not required
headers or a server completion deadline. A client timeout does not prove failure
or authorize a new operation with a new key.

## 4. Exports

An export resolves identifiers and exports matching product records. The same
authentication, request validation, replay and polling rules apply. Products
define the category content in their JSON export parts; v1 does not prescribe
a record schema. Clients must not assume the fixture's example record schema.

On completion, `parts` lists `{ "partNumber", "sha256", "bytes" }`. Each number
identifies a part at the part route. Each `sha256` is lowercase hexadecimal
SHA-256 of the exact downloaded bytes. `bytes` is their length, at most
`maxExportPartBytes`. Return bounded JSON parts with `application/json`; split
larger exports rather than returning an unbounded response. The receipt signs
the same part metadata. Verify the receipt first, then compare each downloaded
part's byte length and digest against its signed entry before using it. Do not
parse and re-serialize a part before hashing it.

Before completion, part GET returns HTTP 409 `data_subject_export_not_ready`.
Expired parts return HTTP 410 `data_subject_export_expired`. Unknown operations,
operations of the other kind and operations owned by another client return
HTTP 404 `data_subject_operation_not_found`, including on the part route. A
`notFound` export has a signed receipt and no parts. A failed export has no
receipt and offers no completed export proof.

Parts expire after a configurable period, default seven days. Clients download
them promptly after completion. The operation and receipt can remain available
after the bytes expire. Neither a still-valid receipt signature nor a retained
part digest promises that the part is still downloadable.

## 5. Receipts and key rotation

A receipt is a DSSE v1 envelope:

```json
{
  "payloadType": "application/vnd.baukit.data-subject.receipt.v1+json",
  "payload": "<standard base64 of receipt JSON bytes>",
  "signatures": [{ "keyid": "<key ID>", "sig": "<standard base64 signature>" }]
}
```

Use standard RFC 4648 base64 with padding when needed, not base64url. The
decoded `sig` is a 64-byte Ed25519 signature. The input to Ed25519 is the DSSE
v1 pre-authentication encoding, PAE:

```text
"DSSEv1" SP decimal(byte_length(type)) SP type SP decimal(byte_length(body)) SP body
```

`SP` is one ASCII space, `0x20`. `type` is the UTF-8 `payloadType` string;
`body` is the decoded payload bytes. Lengths are decimal byte lengths without
leading zeroes, not character counts or base64 lengths. There is no separator
after `body`, and no extra newline. Sign PAE directly with Ed25519, without an
extra prehash.

`keyid` is lowercase hexadecimal SHA-256 of the Ed25519 public key's X.509
SubjectPublicKeyInfo DER bytes, SPKI DER. Do not hash the PEM text, raw 32-byte
public key, private key or certificate. Discovery publishes the same value as
`keyId`, `algorithm: "ed25519"` and the SPKI PEM with the `PUBLIC KEY` label.
Ed25519 SPKI uses OID `1.3.101.112` with absent parameters.

The decoded receipt payload is:

```json
{
  "receiptType": "baukit.data-subject.receipt.v1",
  "contractVersion": "1",
  "operationId": "00000000-0000-4000-8000-000000000001",
  "kind": "erasure",
  "status": "completed",
  "subjectReference": "client-opaque-reference-01",
  "service": { "name": "billing", "version": "4.2.0" },
  "createdAt": "2026-10-20T10:00:00Z",
  "completedAt": "2026-10-20T10:00:01Z",
  "categories": [{ "category": "invoices", "action": "erased", "count": 3 }],
  "restrictions": [],
  "parts": []
}
```

The Baukit signer emits compact UTF-8 JSON with no insignificant whitespace or
trailing newline. Top-level field order is exactly the example's order. Nested
field order is `name, version` for service; `category, action, count` for category
entries; `category, reason, legalBasis, retainUntil` for restrictions; and
`partNumber, sha256, bytes` for parts. Arrays preserve caller order. All fields
are present, including null `retainUntil`. Integers use decimal notation; UUIDs
use lowercase hyphenated form. Strings escape quotes and backslashes as `\"`
and `\\`. Backspace, tab, newline, form feed and carriage return use `\b`, `\t`,
`\n`, `\f` and `\r`. Other control characters through U+001F use lowercase
`\u00xx`; slashes and other Unicode characters remain unescaped UTF-8.
Timestamps are UTC with `Z`, with zero,
three, six or nine fractional digits as required to preserve their precision.
This stable serialization is not RFC 8785 or a requirement for other signers.

Verifiers check the expected media type, decode base64 and construct PAE over
the received payload bytes. They never re-serialize before verification. Reject
unknown key IDs, empty signature lists and envelopes with no valid trusted
signature. The core requires every listed key ID to be known and every base64
signature to be well formed, and accepts at least one valid trusted signature.
After verification, parse the payload and check `receiptType`, `contractVersion`
and a receipt-bearing terminal status. The core also checks reference syntax
and that completion does not precede creation. Callers match service identity,
operation ID, kind, reference and result to their expected operation. A valid
signature alone does not establish those expectations or prove the product's
deletion inventory is complete.

For rotation, publish the new public key as `current`, move the old key to
`previous`, then sign new receipts with the new key. Keep all previous keys
needed for receipts still retained. Allow for the five-minute discovery cache;
a client may refresh discovery once on an unknown ID and retry verification,
but must never accept an unadvertised key. A verifier accepts both current and
previous keys. Removing a key before its receipts expire breaks verification.
Callers storing evidence longer than service retention also retain the trusted
public keys they used to verify it.

## 6. Authentication and errors

Callers are machines using OAuth 2.0 client-credentials access tokens issued by
the product's identity provider. In Baukit, existing `baukit-auth` middleware
establishes a `Principal`. Accept it only when `client_id()` is in the configured
allowlist, it is not an API token and `scopes()` contains the operation's scope.
An implementation without Baukit must enforce the same checks on its verified
principal. Discovery is the only unauthenticated route.

No principal returns HTTP 401. A disallowed client, absent client identity or
API token returns HTTP 403 `data_subject_client_not_allowed`. A missing scope
returns HTTP 403 `insufficient_scope`. Operations belong to their creating
client. An allowed client cannot read another client's operation; an erasure
scope grants no access to exports, and vice versa. Check ownership and kind
before revealing status or export availability.

Every HTTP error uses the `baukit-http` envelope:

```json
{
  "error": {
    "code": "data_subject_request_invalid",
    "message": "The data subject request is invalid",
    "requestId": "opaque-request-id",
    "details": {}
  }
}
```

| Code | HTTP status | Meaning |
| --- | --- | --- |
| `data_subject_idempotency_key_invalid` | 400 | Required key is missing, repeated or malformed. |
| `data_subject_request_invalid` | 422 | Bad reference, identifier count, length, empty normalized value, malformed type or phone. |
| `data_subject_identifier_type_unsupported` | 422 | Valid type was not listed in discovery. |
| `data_subject_idempotency_conflict` | 409 | Same key was used with a different normalized body. |
| `data_subject_operation_not_found` | 404 | Operation is unknown, another kind or another client's. |
| `data_subject_export_not_ready` | 409 | Part requested before completion. |
| `data_subject_export_expired` | 410 | Part is past retention. |
| `data_subject_client_not_allowed` | 403 | Principal is not an allowed machine client. |
| `insufficient_scope` | 403 | Principal lacks the operation scope. |
| `data_subject_operation_failed` | 200, in `error.code` on a failed operation | Accepted work exhausted attempts, failed permanently or exceeded its identity-deletion deadline. No receipt is issued. |

Messages are safe fallback copy, `details` is a safe JSON object, and `requestId`
connects the response to diagnostics. Never include identifiers, credentials,
tokens, key material or internal provider bodies in errors or logs. The core's
typed validation errors expose `code()` and numeric `status_code()` for adapters.
Cryptographic failures are local verification errors, not additional HTTP codes.

## 7. Retention and product adoption

Operation rows and receipts are purged after a configurable period, default
400 days. Export parts have their separate seven-day default. Document the
configured periods for callers. Replay guarantees depend on retained operation
records; clients must reconcile ambiguous requests within that retention period.

Baukit products add `baukit-data-subject` at the release train's exact version.
Clients use `default-features = false`. `ReceiptSigner` accepts an unencrypted
Ed25519 PKCS#8 private key in PEM or DER. Products supply this key through their
secret configuration and publish only the derived public discovery entry.
`ReceiptVerifier::from_discovery` checks key IDs, unique keys and exactly one
current key before accepting envelopes.

The server uses a stable secret of at least 32 bytes for HMAC-SHA256. Key hashes
use the `data-subject-key\0` domain followed by length-prefixed client ID, kind
and idempotency key. Request hashes use `data-subject-request\0` followed by the
length-prefixed serialized `NormalizedRequest`. Lengths are unsigned 64-bit
big-endian byte counts. Each hash is 32 bytes. A database uniqueness constraint
and transaction advisory lock serialize concurrent requests for the same client,
kind and key. The pending operation and job enqueue commit together.

`DataSubjectProduct::resolve` runs during POST. The durable worker runs `erase`
for every resolved subject in one transaction with the category report, fence
and successful lease-owned job transition. With retain mode or no authentication
subject it signs the result in that same transaction. Product methods must not
commit or contact external systems. A product failure rolls back every subject,
fence and result, then the job runner retries with its normal backoff. The
migration's job failure trigger also handles runner timeouts and expired final
leases. Maintain a deletion inventory and return safe, accurate category reports
and legal restrictions. Subjects without authentication accounts need no fence.

For authentication subjects the worker calls
`ErasureService::fence_as_operator_in_transaction` in that same transaction.
It uses the same keyed subject lock, fence and `IdentityRetention` as profile
erasure. Products must continue enforcing `reject_fenced_subject` on sessions
and `guard_subject` when resolving or creating profiles. Old or undated sessions
fail with `profile_erased`; another subject's sessions remain valid. In retain
mode, a login strictly after the fence can re-onboard after completion.

Delete mode durably queues the existing `identity.account.delete` job and
requires the product's `IdentityDeletionHandler`. The operator entry point
does not make an inline provider call because its caller owns the transaction.
The data subject operation stays `pending` without a receipt while any linked
identity deletion is pending. Its safe category report may already describe
committed product deletion. A `data-subject.finalize` job checks the linked
profile-erasure operations after one second, then doubles the delay between
checks up to 60 seconds. Its payload contains only the operation ID and check
count. Waiting schedules the next check and succeeds; a database failure uses
normal job retries. Polling does not consume the attempts reserved for failures.

`DataSubjectConfig::identity_deletion_timeout` defaults to 24 hours and must be
positive and no longer than 30 days. The local erasure transaction records the
wait's start when erasure finishes. Every check uses that timestamp for the
deadline, and no check is scheduled beyond it. A check at or after the deadline
makes the operation `failed` with `data_subject_operation_failed` and no receipt,
including when the product never registered an identity deletion worker.

Only after all linked identity deletions complete does finalization sign and
commit the terminal result. A failed identity operation makes the data subject
operation `failed` with `data_subject_operation_failed` and no receipt. The
product rows stay erased after provider failure or timeout; the fence remains
active. Identity deletion jobs remain queued after a timeout. Later provider
success cannot change the failed data subject operation. Retain failed identity
jobs for operator repair. This separate signing transaction is necessary in
delete mode. The provider cannot see the queued job before product
erasure commits, and a completed receipt must not precede provider success.
The existing self-service DELETE route is unchanged.

Schedule `PostgresDataSubjectStore::purge_expired` in a product recurring job.
Each call accepts 1 to 10,000 rows, removes only operations whose `purge_after`
is before the supplied time, and removes their erasure and finalization payloads
in the same transaction. Use `baukit_jobs::FixedUtcInterval` and slot idempotency
keys to schedule recurring batches. Receipt retention starts at acceptance.
Identity fences and identity deletion jobs follow the separate profile-erasure
policy and are not removed by this purge. The crate README has a mounting example.

## 8. Conformance

The shared fixtures pin discovery, SPKI-derived key IDs, PAE, canonical receipt
bytes and envelopes for `completed`, `notFound`, `restricted` and export with
parts. Negative cases cover tampering, the wrong media type and unknown keys.
Python and OpenSSL generate these fixtures independently of the Rust signer.
An additional signed payload changes field order and whitespace to catch a
verifier that re-serializes. Other implementations should run the same fixtures.

Enable `baukit-test`'s `data-subject` feature for the controlled client-test target
`DataSubjectTargetServer`. `start()` binds an ephemeral loopback socket;
`base_url()`, `discovery()` and `receipt_key()` expose the connection and public
verification inputs. `register_token(bearer, client_id, scopes)` returns a
`DataSubjectTokenId` for request observations. Tokens for the same client share
operation visibility and idempotency records.

`set_script(subject_reference, script)` and `set_default_script(script)` accept
`DataSubjectOutcome` directly or a `DataSubjectScript` with `DataSubjectFault`s.
New POSTs return pending. The first status GET completes an ordinary outcome;
`PendingFor { polls, then }` adds pending status GETs. Replays and downloads do
not advance work. `push_fault(subject_reference, fault)` adds faults to an
explicit script after acceptance for polling and download tests. Receipt faults
use the real signer, so verification fails at the intended check. A signed wrong
reference still passes cryptographic verification and must fail the client's
operation matching.

`rotate_key()` publishes a new current key and retains previous keys for old
receipts. `start_with_options(DataSubjectTargetOptions)` configures advertised
bounds and retention; `advance_time(duration)` tests expiry and purge without
sleeping. `received_requests()` returns method, path, idempotency key, token
registration handle and parsed body. These private observations stay in memory
and omit `Debug` and serialization. Dropping the target shuts down the socket
and delayed responses. See the [`baukit-test` usage example](../../rust/crates/baukit-test/README.md#data-subject-client-tests).

The service-side `check_data_subject_conformance` entry point is planned in T2.
T2 will document its final signature and product adapter requirements here.

## 9. Acceptance checks

- Discovery is public, cacheable for five minutes and advertises supported
  operations, types, bounds, scopes and exactly one current signing key.
- Identifier bounds, normalization, custom syntax and type allowlisting match
  section 2. Identifiers never enter durable operations, jobs, receipts or logs.
- The two operation kinds enforce the machine allowlist, correct scopes,
  creating-client ownership and kind isolation on every protected route.
- POST replay survives lost responses, preserves operation IDs and receipts,
  and returns 409 for a different normalized body under the same key.
- Pending and failed resources carry no receipt. Every other terminal state
  carries a verifiable receipt with the echoed reference and accurate categories.
- Restricted erasure reports each retained category and its legal obligation.
  No-match results produce a signed `notFound` receipt.
- Export parts fit the advertised bound, match signed digests and sizes, reject
  downloads before completion, and return 410 after expiry.
- Independent receipt fixtures verify; tampered payloads, wrong media types and
  unknown key IDs fail. Verification accepts original payload bytes regardless
  of JSON formatting. Rotation preserves verification of retained receipts.
- Errors use the standard envelope and exact codes. Retention and purge remove
  export parts, operations and receipts according to their configured periods.
