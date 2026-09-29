# `@baukit/data-contracts`

Runtime-neutral data contracts, measurement helpers, and executable adapter conformance tests.

The package defines JSON key/value storage, ID-ordered record storage with bounded keyset pagination, atomic transaction callbacks, and schema metadata/migration conventions. It deliberately contains no product entities and no Expo SQLite, Dexie, or Node database adapters. The one storage adapter it ships is `WebStorageKeyValueStore`, because Web Storage needs no dependency.

## Using the contracts

```ts
import type { StoredRecord, TransactionalStorageStore } from '@baukit/data-contracts';

interface Note extends StoredRecord {
  title: string;
}

type NoteDatabase = TransactionalStorageStore<Note>;
```

`TransactionalStorageStore` makes nesting and lifecycle behavior explicit.
Inside a callback, call `withTransaction` on the transaction-scoped context to
join the ambient transaction. Calls on the root store are independent and must
be serialized by the adapter. After `close()` resolves, all operations reject
with `StorageError.code === "storage_closed"`. Quota failures use
`storage_quota_exceeded`; callers never need to parse provider error text.

`KeyValueStore.clearPrefix(prefix)` deletes every key that starts with `prefix`, for example all
drafts of one account on sign-out when keys look like `draft:<account>:<document>`. Matching is
exact and case-sensitive by UTF-16 code units, no character is a wildcard, and an empty prefix
clears the store. `clear()` still empties the whole store.

`WebStorageKeyValueStore` is a `KeyValueStore` over `sessionStorage` or `localStorage`. It stores
each value as JSON text under a namespace you pass, so `clear()` and `clearPrefix()` never remove
other keys of the origin. An empty namespace throws a `RangeError`. Storage failures reject the
returned promise, and a full storage rejects with `StorageError.code === "storage_quota_exceeded"`.

```ts
import { WebStorageKeyValueStore } from '@baukit/data-contracts';

const drafts = new WebStorageKeyValueStore(sessionStorage, 'app:drafts:v1:');
await drafts.set('note:new', { title: '' });
await drafts.clearPrefix('note:');
```

The older `StorageTransaction` and `Transaction` interfaces remain available
for adapters that implement only the original surface.

## Resource-budget measurements

Import production measurements from the `/limits` subpath. Checks return the measured and allowed
values, or throw `LimitExceededError` with those same fields. Products choose each allowed value and
map the error into their own reason code.

```ts
import {
  checkCompactJsonUtf8Bytes,
  checkTrimmedUnicodeScalars,
} from '@baukit/data-contracts/limits';

checkTrimmedUnicodeScalars('  e\u0301  ', 2);
checkCompactJsonUtf8Bytes({ value: 'é' }, 14);
```

Text measurement trims Unicode `White_Space` scalars at both ends. It does not normalize text.
Compact JSON measurement accepts null, booleans, finite numbers, scalar-only strings, dense arrays,
and plain objects. Plain objects may use enumerable own string keys. Non-enumerable properties are
ignored. Symbol keys, accessors, custom prototypes, circular references, unsupported values,
non-finite numbers, and unpaired surrogates throw `ResourceMeasurementError`. The compact encoder
uses `JSON.stringify` property order, though property order cannot change the measured byte count.

Existing product helpers can migrate one call at a time. Replace `codePointLength(value.trim())`
with `trimmedUnicodeScalarCount(value)`, and replace a `JSON.stringify` plus `TextEncoder` byte count
with `compactJsonUtf8Bytes(value)`. Unlike raw `JSON.stringify`, the helper rejects values that JSON
would omit or replace with `null`.

### Limits policy files

`parseLimitsPolicy(value, schema)` validates a product's limits policy file, such as the
`limits.json` the templates generate. The schema names the supported version and the keys of each
section. The parser requires a string `$comment`, the exact version, the exact section and key sets,
and positive safe integers. Keys listed in `allowZero` as `section.key` also accept zero. Any other
shape throws `LimitsPolicyError`, whose message names the failing path but never a value. Pass the
schema `as const` so the result type has one `number` property per declared key.

`enforceLimit(field, reason, check)` runs one check and turns `LimitExceededError` into
`LimitError`, which carries `reason`, `field`, `measured`, and `allowed`. Other errors pass through.

```ts
import { checkTrimmedUnicodeScalars, enforceLimit, parseLimitsPolicy } from '@baukit/data-contracts/limits';

const schema = { version: 1, sections: { text: ['max_characters'] } } as const;
const policy = parseLimitsPolicy(limitsJson, schema);

enforceLimit('title', 'text_too_long', () =>
  checkTrimmedUnicodeScalars(title, policy.text.max_characters),
);
```

Cross-field rules, such as one limit that must not exceed another, stay in product code after the
parse.

## Import envelopes

The `/import-envelope` subpath separates import safety from a product's file format. The product
decoder returns a safe context and an iterable of named row objects. `prepareImportEnvelope` checks
the source byte limit before calling that decoder, stops when the row limit is exceeded, rejects
fields outside the allowlist, checks strings nested inside allowed fields, decodes each row, and
runs the product's preview planner. The planner receives decoded rows but no transaction adapter.

```ts
import {
  commitImportEnvelope,
  prepareImportEnvelope,
} from '@baukit/data-contracts/import-envelope';

const preview = await prepareImportEnvelope({
  source,
  limits: { maxSourceBytes: 2_097_152, maxRows: 5_000, maxStringBytes: 8_192 },
  decodeEnvelope: decodeProductExport,
  fieldAllowlist: { notes: ['id', 'title', 'body'] },
  decodeRow: decodePortableRow,
  plan: planImport,
});

await commitImportEnvelope({
  preview,
  transaction: repositories,
  write: writeImportPlan,
  afterCommit: () => requestSync(),
});
```

The decoder still owns format and schema versions, required fields, duplicate IDs, tombstone
policy, and entity decoding. The planner owns conflicts, provenance, and deletion order.
`commitImportEnvelope` puts the complete product write behind one `withTransaction` call. It invokes
`afterCommit` only after that call resolves. Keep `afterCommit` idempotent because its failure does
not roll back the committed transaction.

This API is additive. Existing import code can migrate by wrapping its current decoder and preview
planner first, then moving the write loop into `commitImportEnvelope`. It does not require a file
format or database migration.

## Exports

The `/export` subpath is the counterpart of the import envelope. `encodeCsv` turns rows into an RFC
4180 CSV string: every record ends with CRLF, and a cell is quoted when it contains a comma, a double
quote, CR, or LF. A row holding one empty cell is written as `""` so a reader still sees one field.
The helper has no runtime dependencies; the product writes the string to a file and shares it.

```ts
import { csvNumeric, encodeCsv } from '@baukit/data-contracts/export';

const csv = encodeCsv(
  [
    ['note', 'amount', 'price'],
    ['=HYPERLINK("https://example.test")', -5, csvNumeric('1.50')],
  ],
  { byteOrderMark: true },
);
```

Formula neutralization is on by default. A text cell gets a leading apostrophe when its first
character is `=`, `+`, `-`, `@`, tab, or CR, or when `=`, `+`, `-`, or `@` follows leading spaces,
tabs, CR, or LF. Only these ASCII characters are checked. The apostrophe changes the value, so a
file that your own importer reads back must either strip it or be written with
`neutralizeFormulas: false`. Use the opt-out only for machine-read files that never open in a
spreadsheet.

Text that looks like a negative number, such as `'-5'`, is neutralized. Pass real numbers as a
finite `number` or as `csvNumeric(text)` when the formatting matters, for example `'1.50'`. Numeric
cells must match the JSON number grammar and are written unchanged. `null` is an empty cell.
`byteOrderMark: true` starts the output with U+FEFF for spreadsheet programs that need it to detect
UTF-8.

By default `null` and `''` both write an empty field, so a reader cannot tell them apart. Two options
keep them distinct for a file your own importer reads back:

- `quoteAllCells: true` quotes every text and numeric cell. A null cell stays unquoted, so `''`
  becomes `""` and `null` stays empty. A row holding only one null cell still writes `""`.
- `nullMarker: '\\N'` writes the marker unquoted for a null cell and quotes a text cell with the same
  content, so `"\N"` reads back as text. The marker must be non-empty and free of double quotes,
  commas, CR, and LF; `encodeCsv` throws a `RangeError` otherwise.

```ts
encodeCsv([[null, '\\N', '', 'x']], { nullMarker: '\\N' }); // \N,"\N",,x
encodeCsv([[null, '', -5]], { quoteAllCells: true }); // ,"","-5"
```

`CsvEncodeError.code` is `invalid_numeric_cell` for a non-finite number or a numeric cell outside
the JSON grammar, `invalid_unicode` for a string with an unpaired surrogate, or `unsupported_cell`
for any other runtime value. The error carries `rowIndex` and `columnIndex` and never the cell
value. `baukit_core::export::encode_csv` in Rust passes the same vectors
(`fixtures/export-csv/csv-encoding-v1.json`).

`ShareOutcome` names what happened after the product handed the file to the platform:

- `shared`: a share sheet or the Web Share API accepted the file. Some native share sheets report
  this even when the user closed them without picking a target.
- `saved`: the file was written to a place the user can open, such as a browser download.
- `cancelled`: the user dismissed the share or save UI and the platform said so, for example with a
  Web Share `AbortError`.
- `unavailable`: the runtime has no share or save mechanism, so nothing was attempted.
- `failed`: a mechanism was attempted and reported an error other than cancellation.

`SHARE_OUTCOMES` lists the same values. The package ships no Expo or browser implementation yet.

## Revisioned writes

The `/revisioned-writes` subpath sends edits of one server document one at a time, each checked
against a revision. It has no React dependency. Wrap it in a hook with `useSyncExternalStore`: the
snapshot object only changes when state changes.

```ts
import { createRevisionedWriteQueue } from '@baukit/data-contracts/revisioned-writes';

const queue = createRevisionedWriteQueue({
  initial: { scope: accountId, document: noteId, acknowledgedRevision: loaded.revision },
  write: async ({ scope, document, value, expectedRevision, afterUnknownOutcome, signal }) => {
    const response = await api.saveNote(scope, document, value, {
      ifMatch: expectedRevision,
      idempotencyKey: await keyFor(document, value, expectedRevision),
      signal,
    });
    if (response.status === 409) {
      return { kind: 'conflict', currentRevision: response.revision, conflict: response.body };
    }
    if (!response.ok) {
      return { kind: 'rejected', error: response.problem };
    }
    return { kind: 'accepted', revision: response.revision };
  },
  signal: pageLifetime.signal,
});

queue.enqueue(editedValue);
await queue.flush();
```

The snapshot keeps two values apart. `acknowledgedRevision` is the last revision the server
confirmed. `unsent` is the local value no write has carried yet. A write sends the value it took
from `unsent` with the acknowledged revision as `expectedRevision`. Edits made while that write is
in flight go into a fresh `unsent` and the status stays `dirty`. An acknowledgement for the older
value never marks the newer one saved. Queued edits are combined with `coalesce(older, newer)`,
which keeps the newer value by default. Pass your own function when a write carries a patch instead
of the whole document.

`enqueue` never sends. Debounce in the product and call `flush`. Concurrent `flush` calls share one
drain, which resolves with the snapshot once the queue is idle or paused.

The write callback reports one of three outcomes:

- `accepted` stores the new revision and sends the next unsent value, if any.
- `rejected` means the server refused the write before accepting it. The value goes back into
  `unsent`, merged with newer edits, and the status is `failed`. `flush` or `retry` sends it again.
- `conflict` pauses the queue with the server's revision and conflict payload. Nothing more is sent
  until the product calls `reset` with a revision it trusts. To keep the local value, `reset` to
  the current server revision and `enqueue` the value again.

A throw from the write callback, or a rejected promise, means the outcome is unknown. The server may
have applied the write and lost the response. The queue moves to `unknown-outcome` and keeps the
exact value and expected revision in `uncertain`, apart from newer edits. `flush` does nothing in
that state. `retry` sends the same value and revision with `afterUnknownOutcome: true`, so the
product can reuse its idempotency key. Otherwise read the server and call `reset`. The queue never
treats an unknown outcome as a failure that happened before the server accepted the write.

`reset(next)` is the generation fence for an account or document switch. It aborts the current
write through the `signal` passed to the callback, drops unsent and uncertain state, and ignores
any late result of the old generation. `cancel()` does the same and keeps the queue in `cancelled`
until the next `reset`. Aborting the injected `signal` option calls `cancel`.

Status is `cancelled`, `writing`, `failed`, `unknown-outcome`, `conflict`, `dirty`, or `idle`, in
that order of precedence. `error` holds the `rejected` error or the thrown value.

## Durable drafts

The `/durable-draft` subpath keeps an unsent form value in any `KeyValueStore`, so it survives a
reload or crash. It has no React dependency.

```ts
import { createDurableDraft, type DraftCodec } from '@baukit/data-contracts/durable-draft';

const noteCodec: DraftCodec<NoteDraft> = {
  version: 2,
  encode: (value) => ({ body: value.body, baseRevision: value.baseRevision }),
  decode: (value, version) => decodeNoteDraft(value, version),
};

const draft = createDurableDraft({
  store,
  key: (scope: { accountId: string; noteId: string }) =>
    `note-draft:${scope.accountId}:${scope.noteId}`,
  codec: noteCodec,
});

await draft.open({ accountId, noteId }, { body: loaded.body, baseRevision: loaded.revision });
draft.update({ body: editedBody, baseRevision: loaded.revision });
await draft.save();
```

Storage holds `{ version, value }`. The codec owns the value: `encode` returns JSON, and `decode`
receives unchecked JSON plus the stored version and returns `decoded`, `corrupt`, or
`unsupported-version`. Validate every field in `decode`. A decoded value from an older version
counts as upgraded, so the draft opens dirty and the next `save` rewrites it. The helper returns
`unsupported-version` without calling `decode` when the stored version is above `codec.version`,
and `corrupt` when the envelope is malformed or `decode` throws. Keep a server base revision or ETag
inside the value when the product needs it for a later conflict check.

An open snapshot has two groups of state. `recovery` says what `open` found: `none`, `restored`,
`corrupt`, `unsupported-version`, or `unavailable` when the read failed. `persistence` says what
storage is doing now: `loading`, `idle`, `saving`, `clearing`, or `failed`. `dirty` is true when
the value differs from what storage holds, and `localRevision` counts local changes. A restored
draft can also be dirty.

`open` never writes or deletes. While recovery is `corrupt` or `unsupported-version`, `save`
resolves `blocked` so the unreadable data stays in place. Call `clear({ reason: 'discarded' })` to
remove it.

`clear` takes the reason for the deletion:

- `{ reason: 'submitted', localRevision }` reports that the server confirmed the value at that local
  revision. The snapshot shows `submission: 'confirmed'` and keeps the value visible. If the user
  edited after that revision, the stored draft stays and `clear` resolves `newer-edits-kept`.
- `{ reason: 'discarded' }` deletes the draft and returns the value to the one passed to `open`.

A failed deletion rejects with a `DraftPersistenceError` and stays in the snapshot as
`persistence: 'failed'`. After a submission, `submission` stays `confirmed`, so the product can tell
"the server has it, but the local copy may come back" apart from "nothing was sent". Read, write,
and delete failures use `DraftPersistenceError` with `code: 'draft_persistence_failed'` and an
`operation`. The message never includes the key or the value.

`open` for a new scope fences the previous one. Storage work of the old scope that had not started
is skipped and resolves `stale`. Work that already started finishes but does not change the new
snapshot. All storage work runs in order, so switching from A to B and back to A reads what A's last
write left. `close()` does not save; call `save` on `pagehide` or when the app moves to the
background.

`move(scope)` hands the open draft to another scope. The usual case is a "new note" draft whose
server create just succeeded:

```ts
await draft.clear({ reason: 'submitted', localRevision });
await draft.move({ accountId, noteId: created.id });
```

When the value is dirty or came from storage, `move` writes it under the new key first, replacing
anything stored there. It then deletes the old key in every case. The value, `localRevision`, and
`submission` stay as they were, and later saves go to the new key. A corrupt or unsupported draft
resolves `blocked`. If the write or the delete fails, `move` rejects with a `DraftPersistenceError`,
the draft stays on the old scope, and calling `move` again retries it. `move` does not remember where
a draft went. If a second editor still holds the old scope, the product stores its own pointer from
the old scope to the new one.

`isScopeActive` fences writes when several editors can hold the same scope, for example two tabs
with a shared lease. The helper calls it before every write or delete of `save`, `clear`, and
`move`, for both scopes of a move, when the queued work starts. When it returns false the operation
resolves `stale` and storage is not touched. Reads are not fenced.

```ts
const draft = createDurableDraft({
  store,
  key: noteDraftKey,
  codec: noteCodec,
  isScopeActive: (scope) => lease.holds(scope.accountId),
});
```

Snapshots are read through `getSnapshot()` and `subscribe`, which suit `useSyncExternalStore`. The
stored value itself is always read asynchronously, because `KeyValueStore` is asynchronous over
IndexedDB and SQLite. Render the loading state while `persistence` is `loading`. With
`WebStorageKeyValueStore` that lasts one microtask.

## Authenticated partitions

`deriveScopedStoreName(namespace, subject)` hashes a length-delimited canonical
identity with SHA-256 and returns an opaque name. Browser and Node runtimes use
`globalThis.crypto.subtle`. React Native must install a standards-compatible
Web Crypto polyfill before calling the default helper, or inject a
`ScopedPersistenceDigest`; the generated Expo template injects `expo-crypto`.

Keep `ScopedPersistenceRegistryStore` outside the domain database (for example,
SecureStore or localStorage). `resolveScopedStore` serializes access through one
registry instance, validates all versioned metadata, and only claims a legacy
store when an explicit inspector returns `claimable` or `current-subject`.
Malformed, unknown-version, inconsistent, or digest-mismatched metadata throws
`PersistenceIdentityMismatchError` with code
`persistence_identity_mismatch` before a domain store is opened.
Keep the configured legacy store name available after a successful claim: every
reopen verifies the recorded name against that configuration and fails closed
if it is missing or changed. A store name may belong to only one registry entry,
including across namespaces.

`ScopedPersistenceLifecycle` immediately hides stale handles, closes before it
opens another subject, resets product-provided user-scoped memory, and publishes
only an open/migrated/hydrated partition. Call `handleSessionExpired()` for a
terminal authentication expiry; it closes and blocks without inventing a
subject switch. Products with an older, already-versioned ownership registry
may supply `resolveStore` to retain those database names while adopting the
shared close/reset/publish lifecycle. The compatibility resolver remains
responsible for validating its legacy metadata and failing closed. Use
`recheckServerSubjectBeforeSyncAdoption` immediately before server identity
adoption or an outbox push.

`RecordStore.list` orders immutable string IDs ascending using JavaScript string comparison. Page sizes are limited to `1..MAX_PAGE_SIZE`, and continuation cursors must be opaque to callers. Adapters should implement keyset cursors based on the last returned ID, not numeric offsets, so insertion before a cursor cannot shift later pages.

## Proving an adapter

Vitest helpers are isolated in a test-only subpath. Vitest is deliberately not a peer dependency, so it is never installed in Jest or production consumers. The subpath has no `default` export condition, so a Jest suite cannot resolve it by accident. Install Vitest in an adapter project's development dependencies before importing the subpath, then register the applicable suites:

```ts
import {
  describeImportEnvelopeContract,
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeScopedPersistenceContract,
  describeTransactionContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';

const makeDatabase = () => new MyAdapter();

describeKeyValueContract(() => makeDatabase().keyValues);
describeRecordStoreContract(() => makeDatabase().records);
describeTransactionContract(makeDatabase);
describeTransactionalStorageContract(makeDatabase);
describeScopedPersistenceContract(makeNamedDatabaseAdapter);
describeSchemaMetadataContract(() => makeDatabase().schemaMetadata);
describeImportEnvelopeContract(importEnvelopeOptions);
```

Each factory must return a fresh, empty store. The record suite supplies its
own JSON-shaped `{ id, label, payload }` records. A transaction implementation
must expose a callback-scoped view and make all callback writes visible
together, or none if the callback throws/rejects. The stronger composite suite
also proves callback results, compound and write-plus-outbox-shaped atomicity,
joined reentrancy, quota normalization, close behavior, and concurrent
transaction serialization. The scoped suite proves offline E→F→E record and
outbox isolation, close-before-open ordering, memory reset, legacy claims,
corrupt-registry blocking, server-subject checks, and terminal expiry. Its
adapter factory must reopen the same durable data for the same name. The
included `InMemoryStore` exposes `keyValues`,
`records`, and `schemaMetadata` namespaces and is itself tested by every suite.
The import-envelope suite uses caller-supplied fixtures and a fresh harness. It checks malformed
files, a write-free preview, field filtering, rollback after a halfway failure, and cursor or sync
state changes after commit.

Persistence adapters and product-specific migration logic belong in product or future adapter packages. The production entry point has no runtime dependencies; only the `/vitest` subpath expects the consumer's Vitest installation.
