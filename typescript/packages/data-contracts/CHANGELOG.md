# @baukit/data-contracts

## Unreleased

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

- Document pending wire receipts and local fence confirmations separately. Test pending, completed and failed receipts with the shared Rust vectors.
- Report `pending` after server profile erasure commits but sign-in account deletion is still finishing. Keep local cleanup and sign-out in the committed flow. A pending receipt may have a null operation ID when a `profile_erased` fence confirms the commit without a receipt.

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.

## 0.7.3

### Patch Changes

- Release the coordinated baukit 0.7.3 train.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- 06cc669: Add two `encodeCsv` options that keep a null cell apart from empty text. `quoteAllCells: true` quotes every text and numeric cell and leaves null cells unquoted. `nullMarker` writes a marker such as `\N` unquoted for a null cell and quotes a text cell with the same content; an empty marker or one holding a double quote, comma, CR, or LF throws a `RangeError`. `baukit_core::export::CsvOptions` gains `with_all_cells_quoted` and `with_null_marker`, and both pass five new shared vectors.

  Default output is unchanged. No breaking changes.

- d423b98: Add `DurableDraft.move(scope)`, which hands an open draft to another scope, for example from a "new" draft to the record the server created. It writes a dirty or stored value under the new key, deletes the old key, keeps the value, revision, and submission state, and sends later saves to the new key. It resolves `moved`, `blocked`, or `stale` and rejects with `DraftPersistenceError` on a storage failure, leaving the draft on the old scope.

  Add the `isScopeActive` option to `createDurableDraft`. The helper calls it before every write or delete; when it returns false, `save`, `clear`, and `move` resolve `stale` without touching storage. Reads are not checked.

  Breaking: the `DurableDraft` interface gains the required `move` method, so a product's own implementation of that interface must add it. Code that only calls `createDurableDraft` is unaffected.

- e26045b: Add `KeyValueStore.clearPrefix(prefix)`, which deletes every key that starts with `prefix`. Matching is exact and case-sensitive, no character is a wildcard, and an empty prefix clears the store. `InMemoryKeyValueStore`, `DexieKeyValueStore` (one IndexedDB key range), and the Expo SQLite key-value store (a UTF-8 byte prefix match inside the store's namespace) implement it, and `describeKeyValueContract` checks it, including SQL `LIKE` wildcards, case, emoji, and U+FFFF. The Dexie real-browser suite now runs the key-value contract too.

  Breaking: `KeyValueStore` gains a required method, so a product's own `KeyValueStore` implementation must add `clearPrefix`.

- Release the coordinated baukit 0.5.1 train.
- 403805a: Add `WebStorageKeyValueStore`, a `KeyValueStore` over `sessionStorage` or `localStorage`. It keeps every key under a required non-empty namespace, stores JSON text, rejects instead of throwing on storage failures, and maps a full storage to `StorageError` `storage_quota_exceeded`. It passes `describeKeyValueContract`. No breaking changes.

## 0.5.0

### Minor Changes

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- acaab1b: Add `parseLimitsPolicy`, `LimitsPolicyError`, `LimitError`, and `enforceLimit` to `/limits`. `parseLimitsPolicy(value, schema)` validates a limits policy file against a schema that names the version, the keys of each section, and the keys allowed to be zero. It returns a result typed from the schema. `enforceLimit(field, reason, check)` turns `LimitExceededError` into `LimitError` with `reason`, `field`, `measured`, and `allowed`.

  The web and mobile templates now call these instead of generating their own parser. Products that copied the template's `parseLimitsPolicy`, `LimitsPolicyError`, and `LimitError` can delete them and declare a schema. `LimitError` gains `measured` and `allowed`, and invalid counts now throw the package's `RangeError` message (`measured must be a non-negative safe integer`) instead of the generated `<field> count must be a non-negative integer`.

  No breaking changes to existing package exports.

- Release the coordinated baukit 0.5.0 train.
- d9225d5: Add two opt-in entry points, neither re-exported from the package root and neither depending on React.

  `/revisioned-writes` adds `createRevisionedWriteQueue`. It sends one write at a time with the last acknowledged revision, keeps edits made during a write as a separate unsent value, and combines queued edits with a caller `coalesce` function. The write callback returns `accepted`, `conflict`, or `rejected`. A conflict pauses the queue until `reset`. A throw means the outcome is unknown: the queue keeps the exact value and revision apart from newer edits, and `retry` sends them again with `afterUnknownOutcome: true`. `reset` fences account and document switches and aborts the in-flight write through the injected `AbortSignal`. The snapshot works with `useSyncExternalStore`.

  `/durable-draft` adds `createDurableDraft`, which keeps a form value in any `KeyValueStore` as `{ version, value }` through a versioned codec that validates unchecked JSON. The snapshot reports `recovery` (`none`, `restored`, `corrupt`, `unsupported-version`, `unavailable`), `persistence` (`loading`, `idle`, `saving`, `clearing`, `failed`), `dirty`, and `localRevision`. `clear` takes `submitted` with the confirmed local revision, which keeps newer edits, or `discarded`. A failed deletion stays in the snapshot. Storage failures throw `DraftPersistenceError` without keys or draft content.

  No breaking changes.

- a9fa22e: Add the `/export` entry point, also re-exported from the package root. `encodeCsv` writes RFC 4180 CSV with CRLF record separators and neutralizes spreadsheet formulas by default: a text cell that starts with `=`, `+`, `-`, `@`, tab, or carriage return, or with `=`, `+`, `-`, or `@` after leading spaces, tabs, CR, or LF, gets a leading apostrophe. Pass `neutralizeFormulas: false` to turn this off, and `byteOrderMark: true` to start the output with U+FEFF. Finite `number` cells and `{ numeric }` cells (built with `csvNumeric`) must match the JSON number grammar and are written without neutralization, so `-5` stays a number. Invalid numeric cells, unpaired surrogates, and unsupported runtime values throw `CsvEncodeError` with a code, row index, and column index, never the cell value. `ShareOutcome` and `SHARE_OUTCOMES` name the share or save results `shared`, `saved`, `cancelled`, `unavailable`, and `failed`; no Expo implementation ships yet. `baukit_core::export::encode_csv` passes the same vectors in `fixtures/export-csv/csv-encoding-v1.json`.

  No breaking changes.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- 40882f6: Add bounded import-envelope preparation, atomic commit orchestration, and adapter conformance cases.
- Release the coordinated baukit 0.3.0 train.
- 5472d3d: Add production resource-budget measurements and checks for trimmed Unicode scalars, compact JSON UTF-8 bytes, byte arrays, and collections.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/data-contracts`.
