# Item 7 evidence: replay-safe HTTP mutations

This note covers item 7 of the [cross-product feature plan](../cross-product-feature-plan.md). It
holds the protocol decisions, the eight-product comparison, the conformance cases, and two scratch
runs of product adapters. The protocol itself, without product names, is
[replay-safe mutations](../platform/replay-safe-mutations.md). Steps 1 to 4 are done here. Step 5,
product adoption, is deferred.

## Source revisions

Baukit baseline `9bc9adb`. Products were read at Eigenruhe `f74cebb` (the plan cites `e44ff88`;
only unrelated animation files are dirty), Hebkit `841bf5d`, Leitbild `bd38b33`, Redemut `a782538`,
Runtime Analyzer `d47bfd5`, Schlauzug `31d3f55` (the plan cites `fb280df`), Solo Leveling System
`3461eaf`, and Tiefgang `2d37a06`. `git status --porcelain` was empty for every file cited below.

Schlauzug's idempotency stack, which the plan marked provisional, is committed in `31d3f55`. The
plan asked for a recheck after commit, and this note is that recheck.

## Source product files

Product paths are relative to `/home/patrick/projects/<product>/backend/`.

- Eigenruhe: parser `crates/eigenruhe-api/src/replay.rs:6-64`, storage
  `crates/eigenruhe-postgres/src/replay.rs:7-107`, purge `crates/eigenruhe-postgres/src/retention.rs:89-124`,
  schema `migrations/20260927000001_rest_replay.sql`.
- Hebkit: `rest_mutation_receipts`, `nutrition_mutation_receipts`, and `plan_duplicate_receipts`
  in `crates/hebkit-postgres/src/adapters/postgres/plans.rs:498,925-1106` and `nutrition.rs:3908-3960`;
  fingerprint `mutation_receipts.rs:20-24`; erasure key `crates/hebkit-api/src/adapters/http/handlers.rs:136-148`;
  conflict codes `error.rs:171,201,220,240,304,358,404,417`.
- Leitbild: parser `crates/leitbild-api/src/journal.rs:425-458`, a second parser
  `crates/leitbild-api/src/lib.rs:461-475`, storage `crates/leitbild-postgres/src/journal.rs:693-790`,
  schema `migrations/0016_api_resource_contract.sql:8-20`, clients `web/src/write-intent.ts` and
  `mobile/src/write-intent.ts` (repo root).
- Redemut: parser `crates/redemut-api/src/lib.rs:236-252`, service check
  `crates/redemut-services/src/user_content.rs:1109-1124`, storage
  `crates/redemut-postgres/src/lib.rs:141-296,1221-1283,1507-1570`, calendar cipher
  `crates/redemut-services/src/lib.rs:251-300`, schema `migrations/20260927000000_api_write_contract.sql:6-19`.
- Runtime Analyzer: parser `crates/finops-api/src/routes/common.rs:99-133`, storage
  `crates/finops-postgres/src/idempotency.rs:13-48`, cleanup
  `crates/finops-worker/src/jobs/maintenance_cleanup.rs:5-28`, schema
  `migrations/0028_request_idempotency.sql:1-16`, client `web/src/lib/admin-actions.ts:32-70`.
- Schlauzug: parser `crates/schlauzug-api/src/lib.rs:1957-2016`, cipher
  `crates/schlauzug-services/src/idempotency.rs:40-123`, storage
  `crates/schlauzug-postgres/src/idempotency.rs:28-111`, pruner `crates/schlauzug-bin/src/bin/api.rs:311-335`,
  schema `migrations/0016_idempotency_replays.sql:3-15`, client
  `packages/game-client/src/mutation-attempt.ts` (repo root).
- Solo Leveling System: parser `crates/sl-api/src/v1/quests.rs:627-660`, claim SQL in eight
  `crates/sl-postgres/src` modules (13 insert sites), boss replay `narratives.rs:920-1006`, purge
  `user_quests.rs:774`, schema `migrations/0030_quest_mutation_replay.sql:2-14` and `0034`.
- Tiefgang: parser `crates/tiefgang-api/src/mutation_headers.rs:4-17`, fingerprint
  `crates/tiefgang-services/src/mutation.rs:5-13`, storage `crates/tiefgang-postgres/src/mutation.rs:18-89`,
  sweep `retention.rs:262-290`, erasure `erasure.rs:35-63`, rotation cipher `webhooks.rs:180-259`,
  schema `migrations/20260927000019_mutation_replays.sql:1-12`.

## Observed duplication

Eight products store a replay row next to a mutation. Seven parse `Idempotency-Key` on writes.
Hebkit uses body UUIDs for creates and the header only for erasure. They agree on the header name and on one
transaction for the effect and the record. They disagree on almost everything else.

| Product | Key policy | Grammar and missing key | Scope | Fingerprint | Stored result | Conflict | In flight | Retention and cleanup | Erasure |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Eigenruhe | Required on most writes; body key must match header on two | 1..=128 graphic; 400 `validation_failed` | owner, operation; target in fingerprint | jsonb `Value` equality, no hash; program create uses raw JSON | plaintext domain result | 409 `idempotency_key_reused` | owner revision `FOR UPDATE` | 7 days; looped `SKIP LOCKED` batches of 1000 | cascade plus explicit delete |
| Hebkit | Body `id` or `operation_id` UUID | serde `Uuid` | owner, kind; three tables | SHA-256 of sorted `Value` | plaintext; nutrition returns 200 plus `idempotent_replay` | 409 with four codes | owner revision `FOR UPDATE` | none, by design | cascade |
| Leitbild | Required on four writes | 1..=128 `[A-Za-z0-9._:-]`; 428 `idempotency_key_required`; erasure uses 1..=200 graphic and 400 | user, operation (target folded in) | hex SHA-256 of DTO in field order | plaintext domain result | 409 `idempotency_key_conflict` | user row `FOR UPDATE` | none | cascade plus explicit delete |
| Redemut | Optional on three creates | 1..=128 visible ASCII; 400 `bad_request` | user, operation | SHA-256 of domain input; calendar hashes `()` | plaintext; calendar ChaCha20-Poly1305 with empty AAD | 409 `idempotency_conflict` | advisory lock or user row | 7 days; only the same user's next write deletes | cascade |
| Runtime Analyzer | Optional on four, required on sync | 1..=200 graphic; 400 `validation_failed` | tenant, actor, route string | SHA-256 of sorted `Value` minus two fields | plaintext row JSON | 409 `idempotency_conflict` | advisory lock | 24 h; daily ctid batches of 1000 without a tenant under `FORCE` RLS | tenant cascade; actor has no FK |
| Schlauzug | Required on eight | 1..=128 `[A-Za-z0-9-._~]`; 400 `idempotency_key_required` and `invalid_idempotency_key` | caller, operation, target, key hash | SHA-256 of DTO in field order; two routes hash `{}` | AES-256-GCM, scope in AAD, env key | 409 `idempotency_key_conflict` | claim-first unique index | 24 h; looped pruner, batch 1000 | explicit delete by caller |
| Solo Leveling System | Required on eleven, optional on one | 1..=128 visible ASCII; 400 `invalid_idempotency_key` and `idempotency_key_required` | user, operation | hand-built strings; two constants | plaintext; `response_status` never read | 409 `idempotency_key_reused` | claim-first unique index | 24 h; unbatched delete inside the outbox poll | soft delete, so the cascade never fires |
| Tiefgang | Optional on ten, required on rotate-secret | 8..=128 `[A-Za-z0-9._:-]`; 400 `validation_failed` | user, operation; target in fingerprint | SHA-256 of hand-picked `json!` | plaintext; rotation secret via `baukit-credential-vault` | 409 `idempotency_key_reused` | claim-first unique index | 24 h; one unlooped sweep batch | pseudonymizes, so the cascade never fires |

No product sends a replay marker header, and none stores response headers. Only Solo Leveling
System stores a status, and it never reads it back. Seven of eight write the replay row in the
mutation transaction. Solo Leveling System's boss completion and quest start commit a pending
row, then finish in a second transaction.

On the client, Leitbild (`write-intent.ts`), Runtime Analyzer (`admin-actions.ts`), and Schlauzug
(`mutation-attempt.ts`) keep one key per logical write. Leitbild keeps it on any error, Runtime
Analyzer drops it on a conclusive 4xx other than 408, and Schlauzug drops it unless the status is
below 400, 408, 429, or 5xx. Only Leitbild persists keys across a reload. Solo Leveling System
mints a key per tap, so a retry after a lost response is a new request. No product retries a
`POST` automatically outside an MCP adapter.

## Baukit owner

- `baukit-http` owns the header parser, the four error codes, and the CORS allowlist entry.
- `baukit-test` owns the product-facing conformance check.
- `@baukit/api-runtime` owns the client key store, the outcome classifier, and keyed retries.
- Products keep the replay table, fingerprint input, snapshot, cleanup job, and erasure.

## Public types and errors

`baukit-http`:

- `IDEMPOTENCY_KEY` (`HeaderName`, `idempotency-key`) and `MAX_IDEMPOTENCY_KEY_BYTES` (255).
- `IdempotencyKeyRule::new(min, max)` and `try_new`, with `min_bytes`, `max_bytes`, `parse`,
  `required`, and `optional`. `InvalidIdempotencyKeyRule` for bad bounds.
- `IdempotencyKey<'h>` with `as_str`. `Debug` prints `IdempotencyKey(..)`.
- `InvalidIdempotencyKey { RepeatedHeader, Empty, InvalidCharacter, TooShort, TooLong }` with
  `reason()`.
- `IdempotencyError { Required, Invalid(_), Reused, InProgress }` with `status` and `code`, and
  `From<IdempotencyError> for ApiError`.
- `IDEMPOTENCY_KEY_REQUIRED_CODE`, `INVALID_IDEMPOTENCY_KEY_CODE`, `IDEMPOTENCY_KEY_REUSED_CODE`,
  and `IDEMPOTENCY_KEY_IN_PROGRESS_CODE`.
- `Idempotency-Key` is in the default CORS allowed headers.

`baukit-test`:

- `ReplaySafeMutationAdapter` with `create_owner`, `execute`, `effects`, `replay_records`,
  `expire_replay_records`, `purge_expired`, `erase_owner`, and `discard_transient_state`.
- `ReplayRequest`, `ReplayOperation`, `ReplaySnapshot`, `ReplayOutcome`, `CommitCheckpoint`,
  `InjectedRollback`, and `ReplayConformanceInputs`.
- `check_replay_safe_mutation_conformance`, `assert_replay_safe_mutation_conformance`, and
  `ReplayConformanceError`.

`@baukit/api-runtime`:

- Root: `IDEMPOTENCY_KEY_HEADER`, and `RetryableMethod` now includes `POST` and `PATCH`.
- `@baukit/api-runtime/idempotency`: `createIdempotencyKeyStore`, `IdempotencyKeyStore`,
  `IdempotencyKeyStorage`, `IdempotencyKeyStoreOptions`, `StoredIdempotencyKey`, `MutationIntent`,
  `MutationAttemptOutcome`, `classifyMutationStatus`, `classifyMutationError`,
  `sendIdempotentMutation`, `canonicalJson`, and `IDEMPOTENCY_KEY_IN_PROGRESS_CODE`.

## Decisions

### Key policy per route

Required versus optional is a caller policy, chosen per route with `IdempotencyKeyRule::required`
or `optional`. The products split along one line. Routes that spend a balance, grant a reward,
rotate a secret, or start a job require a key (Schlauzug workshop, Solo Leveling System shop and
rewards, Tiefgang rotate-secret, Runtime Analyzer sync). Plain creates whose duplicate is a
second visible row keep it optional so MCP tools and scripts can call them (Redemut, Runtime
Analyzer budgets, Tiefgang projects). The platform doc turns that into guidance. `PUT` and
`DELETE` of a known ID and `If-Match` writes need no key.

### Grammar

One charset, visible ASCII `0x21..=0x7E`, with caller bounds and `max` at most 255. Every product
charset is a subset of it, so each product keeps its current accepted set by choosing its bounds.
Tiefgang's `8..=128` and Solo Leveling System's `1..=128` both fit. A product that narrows its
charset today (Leitbild, Schlauzug, Tiefgang) widens it on adoption. That is not a break for its
clients, because every key they send still parses. The server hashes the key, so the charset does
not reach storage. The key is opaque. The parser does not read the structured-field string form
the IETF draft uses, since no client sends quotes and treating quotes as data keeps parsing
single-pass.

### Error codes

| Case | Status | Code | Precedent |
| --- | --- | --- | --- |
| Missing | 400 | `idempotency_key_required` | Schlauzug, Solo Leveling System, Tiefgang rotation; Leitbild used 428 |
| Malformed | 400 | `invalid_idempotency_key` with `details.reason` | Schlauzug, Solo Leveling System |
| Reused with other input | 409 | `idempotency_key_reused` | Eigenruhe, Solo Leveling System, Tiefgang |
| Still running | 409 | `idempotency_key_in_progress` | none yet; for a product that sets `lock_timeout` |

428 means a missing precondition header, which in Baukit is `If-Match`. Leitbild's 428 would make
the two indistinguishable by status. The reuse code splits three ways today
(`idempotency_key_reused`, `idempotency_key_conflict`, `idempotency_conflict`) plus Hebkit's four.
The majority name matches the other two codes' prefix. The IETF draft suggests 422 for reuse; all
eight products use 409, so 409 stays.

### Scope, operation, and key storage

The scope is `(caller scope, operation, key digest)`. The operation is a stable name, not Runtime
Analyzer's route string, so a path rename keeps records valid. The target ID goes in the
fingerprint, as Eigenruhe and Tiefgang do, so a reused key on another target is a 409 and not a
second effect. Schlauzug puts the target in the scope instead, which turns the same mistake into
a silent second effect. The server stores a SHA-256 digest of the key, as Schlauzug does.

### Canonical fingerprint

SHA-256 over `serde_json::to_vec` of a `Value` built from the operation, target, expected
revision, and the typed, normalized input. `serde_json` is built without `preserve_order` in every
product and in Baukit, so object keys sort. Three products hash a DTO in struct field order
(Leitbild, Schlauzug) or a hand-built string (Solo Leveling System), and three hash a constant on
some route (Redemut calendar, Schlauzug duplicate and refund, Solo Leveling System daily login and
premium pass), which disables the conflict check there. The Baukit fingerprint helper is deferred;
the reference adapter in `postgres_tests.rs` shows the shape.

### Durable result snapshot

Store the status and body. A replay returns both unchanged. Handlers rebuild `Location` from the
body. A replayed `ETag` can be stale, as Leitbild, Redemut, and Hebkit already accept, and the next
`If-Match` write returns 412. No replay marker header. Hebkit nutrition's 200 plus
`idempotent_replay` is the only marker in eight products, and no client reads it.

### One transaction

The claim, the effect, and the snapshot commit together. Claim-first `INSERT ... ON CONFLICT DO
NOTHING RETURNING` followed by `SELECT ... FOR UPDATE` is the reference shape, from Tiefgang,
Schlauzug, and Solo Leveling System. An advisory or owner-row lock taken before the lookup is also
accepted. Pending-then-finalize across two transactions is forbidden. Solo Leveling System's boss
completion is safe today only because reward grants carry their own dedupe key.

### Retention and expiry

Each product picks a horizon, 24 hours to 7 days in practice. An expired row is invisible and the
next request with that key runs as new, which all six products with a TTL already do. Cleanup is
bounded and looped, as Eigenruhe and Schlauzug do. Under `FORCE ROW LEVEL SECURITY` cleanup needs
a role with a cleanup policy or `BYPASSRLS`; the conformance check catches a cleanup that deletes
nothing. Hebkit's and Leitbild's "keep until erasure" is not allowed. Hebkit keeps receipts so a late
resend cannot recreate a deleted resource. Under the contract a resend after the horizon runs
again, and clients prevent that by keeping their key TTL below the server horizon.

### Erasure and privacy

Account erasure deletes replay rows. Tiefgang and Solo Leveling System have `ON DELETE CASCADE`
but never delete the user row, so their rows survive erasure until TTL. The platform doc says so.
Keys and bodies stay out of logs, spans, and error text; `IdempotencyKey` hides its value from
`Debug`, and `ReplayConformanceError` never prints adapter errors.

### Encryption

Encryption of snapshots is not mandatory. It is required when the snapshot is the only plaintext
copy of a value, which in the survey means Tiefgang's rotated webhook secret, Redemut's calendar
feed token, and Schlauzug's run token. Those products encrypt today with three different schemes:
`baukit-credential-vault` AES-256-GCM with the subscription ID in the AAD and a versioned key
(Tiefgang), ChaCha20-Poly1305 with empty AAD (Redemut), and AES-256-GCM from one env key with no
rotation (Schlauzug). The contract names `CredentialCipher` with the replay row ID as scope,
because it is already in Baukit, binds the ciphertext to one row, and rotates keys.

Encrypting every snapshot was rejected. A dialog or journal snapshot is a copy of rows that sit in
plaintext in the same database, so encryption keeps nothing from someone who can read the table.
It does add a key that must be present on every replay; Schlauzug's key change maps to 500 for up
to 24 hours, and the client keeps retrying because 5xx means possibly committed. Erasure and a
horizon protect ordinary snapshots. Encryption protects the few that hold secrets.

### Client half

The key store keys on canonical JSON of `[account, operation, body]`, so member order in the body
does not matter and account switches never share keys. It keeps the key for `possibly-committed`
outcomes and drops it otherwise. The classifier follows Schlauzug's `retryMayHaveCommitted` rather
than Redemut's, which treats a plain 500 as definite. A 409 `idempotency_key_in_progress` counts
as possibly committed. The store never throws on expiry, which avoids Schlauzug's lock-up where an
`ExpiredMutationAttemptError` has no status and is kept forever. Expiry mints a new key. Storage is
a port, so a web product can pass `sessionStorage` and a mobile product AsyncStorage, as Leitbild
does today.

`createApiFetch` accepts `POST` and `PATCH` in `retry.methods` and retries them only when the
request carries `Idempotency-Key`. An unkeyed `POST` never retries, whatever the options say.

### Relation to revisioned writes

`@baukit/data-contracts/revisioned-writes` decides when to resend a document write, including
after `afterUnknownOutcome`. The key store decides which key an attempt carries. A revisioned
write uses `If-Match` and needs no key. A keyed create sent from the queue's write callback puts
the expected revision, if any, in `intent.body`, so a resend of the same value maps to the same
key. Neither package imports the other.

## Cases

`rust/crates/baukit-test/src/replay_safe_mutation.rs`, in order:

1. Lost response: the retry replays the first snapshot, and the effect count stays 1.
2. Equivalent input: the same input with reordered members replays.
3. Changed input: the same key with other input returns `Conflict`, with no second effect.
4. Simultaneous requests: the first request pauses at `CommitCheckpoint::reached`, the second runs
   for 250 ms, then the first commits. The second must replay the same snapshot or report
   `InProgress`. Exactly one effect.
5. Owner isolation: another owner's same key applies its own effect.
6. Operation isolation: the same key on the secondary operation applies its own effect.
7. Rollback before commit: an injected rollback at the checkpoint leaves no effect and no record,
   and the next attempt applies. The adapter must call the checkpoint and must fail when it fails.
8. Crash after commit: `discard_transient_state` drops caches, and the retry still replays.
9. Expiry: after `expire_replay_records`, changed input applies. Two effects, one record.
10. Bounded cleanup: five expired records with limit 2 purge as 2, 2, 1; a live record survives
    and still replays.
11. Erasure: `erase_owner` leaves no replay records.

Unit tests run the check against an in-memory store with one injected fault per case (14 faults)
and assert each fault is reported with a message that hides adapter details.

Docker tests in `replay_safe_mutation/postgres_tests.rs` run a PostgreSQL reference adapter that
is not exported. It uses an app role and a janitor role, `FORCE ROW LEVEL SECURITY`, an owner
policy on `app.owner_id`, a janitor policy `USING (expires_at < now())`, a key digest, a
fingerprint over sorted JSON, claim-first locking, and cascade erasure. It conforms. A variant
whose cleanup role has no policy fails with exactly the two bounded-cleanup violations.

## Step 2 product runs

A scratch crate outside the repository copied product SQL and Rust verbatim, wired each copy to
`ReplaySafeMutationAdapter`, and ran it against Docker PostgreSQL. Nothing was committed.

- Tiefgang `2d37a06`: `mutation_key`, `claim`, `finish`, the `mutation_replays` migration, and the
  sweep from `retention.rs:269-290`, with erasure copied from `erasure.rs` (pseudonymize, keep the
  user row). Result: one violation, `erasure: erasing an owner left its replay records`. Every
  other case passed, including the simultaneous-request race.
- Runtime Analyzer `d47bfd5`: `request_idempotency` with its RLS policy, the advisory-lock claim,
  and the ctid cleanup from `maintenance_cleanup.rs`. Cleanup as the superuser owner conforms.
  Cleanup as a role without `BYPASSRLS` fails with `bounded cleanup: a batch with limit 2 deleted 0
  records; expected 2` and `bounded cleanup: cleanup left expired records`. Its local compose file runs
  the worker as the superuser database owner, so the defect stays hidden there.

## Failure behavior

- A repeated header is `invalid_idempotency_key` with reason `repeated_header`, not a merged value.
- A non-ASCII byte is `invalid_character`, not `idempotency_key_required` (Schlauzug's bug).
- A reused key with other input is 409 and changes nothing.
- A crash between commit and response loses nothing; the retry replays.
- A rollback leaves no record, so a retry after a 412 or 422 runs fresh.
- On the client, any throw without a status is possibly committed and keeps the key.

## Privacy boundary

The header parser never logs. `IdempotencyKey`'s `Debug` output is `IdempotencyKey(..)`. Error
bodies carry fixed messages and only `details.reason` for malformed keys. The conformance check
prints case names and fixed messages, never adapter errors or snapshots. The client key store
exposes the slot text to its storage port, and the doc tells a persistent adapter to digest it
when bodies are sensitive.

## Supported runtimes

Rust MSRV 1.95, and the `baukit-test` default `postgres:18-alpine` image in the Docker tests. The
client defaults to `globalThis.crypto.randomUUID`, which Node and current browsers provide. A
runtime without it passes `keyFactory`.

## Breaks

- `baukit-http`: `Idempotency-Key` is in the default CORS allowed headers. A product that listed
  it in `additional_allowed_headers` gets a duplicate entry; drop it.
- `@baukit/api-runtime`: `RetryableMethod` includes `POST` and `PATCH`. Before, `methods: ['POST']`
  was filtered out; now it retries keyed `POST` requests. No product passes `POST` today.
- `baukit-test`: additive.

## Template

No template sends or parses `Idempotency-Key`, and the CORS default does not change generated
code. The snapshot trees and the generated fixture are unaffected.

## Deferred adoption

- A `baukit-http` fingerprint and key-digest helper, and an optional SQLx storage helper, wait
  until two products adopt the conformance check and their adapters converge.
- An OpenAPI helper for the `Idempotency-Key` parameter and the 409 responses belongs to item 8,
  which owns `baukit-openapi`.
- Product adoption, step 5, per product below.

## Product adoption change

- Eigenruhe: parse with `IdempotencyKeyRule::new(1, 128)`, rename nothing on the wire for 409,
  hash the fingerprint instead of storing it, replace program create's raw-JSON fingerprint, add
  a conformance adapter.
- Hebkit: keep body IDs for creates; move the erasure key to `IdempotencyKeyRule::new(16, 128)`;
  collapse the four 409 codes to `idempotency_key_reused`; add `expires_at` and a purge.
- Leitbild: replace both parsers with one rule, change 428 to 400, add `expires_at` and a purge,
  move `write-intent.ts` onto `createIdempotencyKeyStore` with its storage port.
- Redemut: move the calendar snapshot to `CredentialCipher`, add a global purge, fix the constant
  calendar fingerprint, send keys from web and mobile creates.
- Runtime Analyzer: run cleanup under a cleanup role policy, use a stable operation name instead
  of the route string, move `admin-actions.ts` onto the key store, run the conformance check.
- Schlauzug: move the snapshot cipher to `CredentialCipher`, put the target in the fingerprint,
  fix the two `{}` fingerprints, replace `mutation-attempt.ts` with the key store.
- Solo Leveling System: collapse the 13 claim copies behind one repository function, make boss
  completion single-transaction, delete replay rows on soft delete, reuse keys per intent on the
  client, batch the purge.
- Tiefgang: delete replay rows in erasure, loop the sweep, adopt `IdempotencyKeyRule::new(8, 128)`
  and the shared codes, add a conformance adapter from the scratch run.

## Product defects found

- Eigenruhe: program create fingerprints raw JSON, so a defaulted field turns a retry into 409;
  the full request body is stored as the fingerprint.
- Hebkit: four 409 codes for one situation, and plan, session, and exercise reuse is
  indistinguishable from an ID unique-violation; receipts never expire; the erasure keyspace is
  global with an unsalted subject hash kept forever.
- Leitbild: two grammars and two missing-key statuses; receipts never expire and keep deleted
  journal bodies in plaintext; the run-start fingerprint ignores `locale`.
- Redemut: no global purge; constant calendar fingerprint; the service's `invalid_idempotency_key`
  is unreachable; the erasure key clients send is ignored; calendar AAD is empty.
- Runtime Analyzer: cleanup under `FORCE` RLS relies on a superuser worker (reproduced above);
  `actor_id` has no FK; the 200-byte key limit is in three places; report replay returns a stale
  `pending` row.
- Schlauzug: the client locks up after `ExpiredMutationAttemptError`; no cipher key rotation;
  a changed body silently drops an uncertain attempt; `{}` fingerprints on two routes; a non-ASCII
  key gets `idempotency_key_required`; the fingerprint depends on struct field order.
- Solo Leveling System: per-tap keys defeat replay; 13 copies of the claim SQL; pending-then-finalize
  for boss completion; unbatched purge in the outbox poll; `response_status` is dead; soft delete
  skips the cascade; constant fingerprints on two routes.
- Tiefgang: erasure leaves replay rows with personal data (reproduced above); the sweep runs one
  unlooped batch; the erasure key on the client is not scoped to the account; a committed row with
  a NULL response is reported as a conflict. The plan calls the fingerprint non-canonical. Every
  caller passes a `json!` value, which serializes with sorted keys, so it is canonical in practice;
  the generic `T: Serialize` signature would allow a struct in field order.
