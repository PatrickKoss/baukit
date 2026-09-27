# SQLite migration conformance and Node test driver evidence

Item 15 of the [cross-product feature plan](../cross-product-feature-plan.md).

## Source revisions

- Hebkit `841bf5d`: `mobile/src/db/sqlite/migrations.ts:1364-1405` (`migrateDatabase`),
  `mobile/src/db/sqlite/adapter.ts`, `mobile/src/db/sqlite/adapters/expo-sqlite.ts:11,33`,
  `mobile/src/db/sqlite/adapters/better-sqlite.ts:9-10,30`.
- Redemut `a782538`: `packages/data/src/migrations.ts:20,293-341` (`LOCAL_MIGRATIONS`,
  `runMigrations`, `validateMigrations`), `packages/data/src/driver.ts`,
  `packages/data/src/expo-sqlite.ts:40-76,101` (`ExpoSqliteDriver`, `openExpoSqlite`),
  `packages/data/src/node-sqlite.ts:5-12,56`, `mobile/src/learning.ts:199`.
- Tiefgang `2d37a06`: `mobile/src/db/sqlite/migrations.ts:3,280-352` (`migrate`),
  `mobile/src/db/sqlite/adapter.ts`, `mobile/src/db/sqlite/adapters/better-sqlite.ts:8,21`.
- Eigenruhe `f74cebb`: `mobile/src/db/contract-tests/better-sqlite.ts:5-34`, its casts in
  `mobile/src/db/contract-tests/persistence.test.ts`,
  `mobile/src/db/contract-tests/session-startup.test.ts`, and
  `mobile/src/features/data-transfer/import-contract.test.ts`.
- `expo-sqlite` 57.0.1: `src/SQLiteDatabase.ts` (`withExclusiveTransactionAsync`,
  `withTransactionAsync`), `src/paramUtils.ts` (`normalizeParams`), and the Android and iOS
  native modules (open options, binding).
- Node 24.20.0 `node:sqlite` with SQLite 3.53.4.

## Baukit owner

`@baukit/data-contracts-expo-sqlite`, new entry point `@baukit/data-contracts-expo-sqlite/testing`
(`typescript/packages/data-contracts-expo-sqlite/src/testing.ts`).

## Step 1: runner comparison

| Behavior | Hebkit `migrateDatabase` | Redemut `runMigrations` | Tiefgang `migrate` |
|---|---|---|---|
| Version record | `schema_migrations(version INTEGER PRIMARY KEY, name, applied_at TEXT)` | `local_schema_migrations(version INTEGER PRIMARY KEY, name, applied_at INTEGER) STRICT` | `PRAGMA user_version = 11`, written at the end, never read |
| `user_version` | not used | not used | overwritten on every start |
| Ordering | sorts the list by version | sorts, then rejects duplicates, zero, and non-integers | none, one fixed schema script |
| Transaction | one per step, SQL and history insert together | one per step, SQL and history insert together | none |
| Fresh install | applies all steps | applies all steps | runs `CREATE ... IF NOT EXISTS` and column probes |
| Upgrade | applies the steps missing from history | applies the steps missing from history | probes `PRAGMA table_info` and adds missing columns |
| Failed step | rolls back that step, earlier steps stay | rolls back that step, earlier steps stay | leaves every statement that ran before the failure |
| Restart | skips applied steps | skips applied steps | re-runs the idempotent script |
| Future version | accepted silently | accepted silently | accepted, and `user_version` is lowered to 11 |
| Foreign keys | `PRAGMA foreign_keys = ON` at open; a step can turn them off around its transaction | `PRAGMA foreign_keys = ON` on the root handle before migrating | `PRAGMA foreign_keys = ON` before the script |
| Device transaction | `withTransactionAsync` on the root connection | `withExclusiveTransactionAsync`, a new connection per step | none |

Redemut runs two step lists against one history table. `openExpoSqlite` applies versions 11 and 12
first, and `learning.ts:199` then applies `LOCAL_MIGRATIONS` (versions 1 to 10 plus 12). A fresh
device therefore applies 11 and 12 before 1 to 10. That works because each list only skips versions
already in history, but a future-version check must compare against the union of both lists, or
the first call refuses every database that the second call has already upgraded.

## Step 2: shared suite and the decision gate

Hebkit and Redemut use the same version semantics: integer versions in a history table, one
transaction per step around the SQL and its history row, and a skip of every recorded version. The
suite never reads that table. It checks observable schema and rows, and it produces the
"newer build" state by running the adapter itself with one extra step, then asks the older step
list to refuse the result. Neither runner needs a switch or a profile, so the gate is met and the
helper ships as one API, `createSqliteMigrationConformanceTests(adapter)`.

The adapter is one function, `migrate(database, steps)`. It receives a `NodeSqliteDatabase` and the
full list of `{ version, name, sql }` steps for one build, which is the shape both runners already
take. Each product wraps the database in its own Expo driver, so the suite exercises the driver
that ships to the device, not a test-only one.

The five cases are fresh install, upgrade with rows kept, restart without re-applying, complete
rollback of a failed step (a step whose third statement fails after an `ALTER TABLE` and a
`CREATE TABLE`), and refusal of a database migrated by a newer build with the file unchanged.

I copied the two runners and their Expo drivers into a scratch directory outside the repository and
ran the five cases against the built `dist/testing.js`. Hebkit's `withTransactionAsync` came from a
scratch shim that runs `BEGIN` and `COMMIT` on the root handle, as Expo does.

| Profile | Fresh | Upgrade | Restart | Rollback | Newer build |
|---|---|---|---|---|---|
| Hebkit as shipped | pass | pass | pass | pass | fail |
| Hebkit, transaction removed | pass | pass | pass | fail | fail |
| Hebkit, newer-version guard added | pass | pass | pass | pass | pass |
| Redemut as shipped | pass | pass | pass | pass | fail |
| Redemut, transaction removed | pass | pass | pass | fail | fail |
| Redemut, newer-version guard added | pass | pass | pass | pass | pass |
| Tiefgang pattern (idempotent DDL, probes, no transaction) | pass | pass | pass | fail | fail |

The guard used in the scratch runs reads `max(version)` from the product's history table before
any step and throws when it exceeds the highest version in the list. It needs no change to the
suite. The package's own tests repeat the same four shapes with a generic history runner and a
generic versionless runner.

Running Tiefgang's real `migrate` on a `NodeSqliteDatabase` confirmed the negative case directly.
After `PRAGMA user_version = 99`, a second `migrate` set it back to 11 without an error. After
replacing `rules` with a view, `migrate` failed with `views may not be indexed` but had already
recreated `user_settings`.

## Step 3: Node driver

### Product drivers compared

| Driver | Library | Surface | Transaction | Foreign keys | Integers | File |
|---|---|---|---|---|---|---|
| Redemut `NodeSqliteDriver` | `node:sqlite` | own `SqliteDriver` port | `BEGIN IMMEDIATE` on the same connection, own queue | on | bound as REAL | `:memory:` default |
| Eigenruhe `BetterSqliteExpoDatabase` | better-sqlite3 | Expo methods, cast with `as unknown as SQLiteDatabase` | `BEGIN IMMEDIATE` on the same connection | library default (on) | bound as INTEGER | `:memory:` only, no `databasePath` |
| Hebkit `BetterSQLiteAdapter` | better-sqlite3 | own `DatabaseAdapter` port | `BEGIN IMMEDIATE` on the same connection | on | bound as INTEGER | caller path |
| Tiefgang `BetterSqliteAdapter` | better-sqlite3 | own `SqliteAdapter` port | `BEGIN IMMEDIATE` on the same connection, nesting joins | on through `migrate` | bound as INTEGER | `:memory:` default |

Only Eigenruhe's driver has the Expo statement surface. All four run transactions on the root
connection, so a root statement during a transaction joins it instead of failing, and all four
enforce foreign keys. On the device neither holds for `withExclusiveTransactionAsync`. None of them
exposes `databasePath`, which the adapter needs since item 1 keyed its queue by file.

### What Baukit ships

`NodeSqliteDatabase` and `NodeSqliteConnection` on Node's built-in `node:sqlite`. Node 24 is the
version in `typescript/.nvmrc` and the package's `engines` floor. On 24.20.0 the module loads
without an experimental warning, `node:sqlite` is in `module.builtinModules`, and it loads under
Jest 29.7, so it needs no dependency, optional or peer. The package gains only `@types/node` as a
dev dependency.

It mirrors the device where the product drivers do not:

- `withExclusiveTransactionAsync` opens a second connection to `databasePath`, runs `BEGIN`, the
  task, and `COMMIT`, rolls back when the task throws, and closes that connection. A root write
  during the transaction fails with `database is locked` (SQLite code 5), because Expo sets no busy
  timeout and the driver opens with `timeout: 0`.
- Every connection opens with foreign keys off and double-quoted string literals on, which are
  SQLite's compile-time defaults on Expo's Android and iOS builds.
- Parameters normalize as `expo-sqlite`'s `normalizeParams` does: variadic values, one array, or one
  object with prefixed keys, and a lone blob is one value. Booleans and safe integers bind as
  64-bit integers, `undefined` as NULL, and an `ArrayBuffer` as a blob.
- Rows are plain objects. `getFirstAsync` returns `null` for no row, and `runAsync` returns
  `{ changes, lastInsertRowId }` as numbers.
- Every call resolves on a later microtask, as a native call does.

`:memory:` and the empty path throw a `TypeError`, because on the device a transaction connection
to an in-memory path sees a separate empty database. With no argument the driver creates a
temporary file and deletes it on `closeAsync()`.

Known differences from the device, documented in the README:

- An unknown named parameter or an extra positional value throws. The device ignores both.
- A root read during an open transaction succeeds and sees the committed state.
- An integer outside the safe range throws instead of binding.
- `runAsync` with several statements runs only the first, as on the device, but `execAsync` is the
  supported way to run a script.
- `withTransactionAsync`, `prepareAsync`, and the synchronous methods are not implemented.
  `withTransactionAsync` shares the root connection on the device, which the Baukit adapter never
  uses.

The package tests run the full shared contract set (`describeRecordStoreContract`,
`describeKeyValueContract`, `describeSchemaMetadataContract`, `describeTransactionalStorageContract`,
and `describeScopedPersistenceContract`) with `ExpoSqliteStore` on `NodeSqliteDatabase`, next to the
existing fake-database run. I also installed the packed tarball in a scratch Jest 29.7 project with
Babel and ran the driver and the conformance cases through `require('@baukit/data-contracts-expo-sqlite/testing')`.

## Step 4: private Expo drivers

Hebkit and Redemut should not move their migrations onto `ExpoSqliteStore`. That adapter is a
namespaced JSON record store with no raw SQL, and both products keep relational schemas (Hebkit 36
migrations with 51 foreign-key references, Redemut 12 versions with 7). What they need from Baukit is the
per-file operation queue that item 1 built, around their own raw statements.

Decision: keep both private drivers for now, and record a follow-up to export a queued raw
connection from the adapter (root statements and exclusive transactions on the same per-file
queue). That is a runtime change to the Expo adapter and needs the device-conformance app, so it
does not belong in this item.

Two product hazards came out of the comparison:

- Foreign keys are not enforced inside `withExclusiveTransactionAsync` on the device. Redemut turns
  them on for the root handle only, so every write in its transactions skips the check. A scratch
  run of Redemut's `runMigrations` and `ExpoSqliteDriver` on `NodeSqliteDatabase` rejected an
  orphan insert on the root handle and accepted the same insert inside a transaction. Hebkit would
  lose enforcement, including its `ON DELETE CASCADE` clauses, if it moved to exclusive
  transactions without running `PRAGMA foreign_keys = ON` inside each one.
- Hebkit's `withTransactionAsync` runs on the root connection, so a root statement from other code
  that arrives during a migration or repository transaction joins it and commits or rolls back with
  it.

## Public types

From `@baukit/data-contracts-expo-sqlite`:

- `ExpoSqliteDatabase`: `databasePath`, `execAsync`, `runAsync`, `getFirstAsync`, `getAllAsync`,
  `withExclusiveTransactionAsync`, `closeAsync`. The `ExpoSqliteStore` constructor takes it instead
  of `SQLiteDatabase`. Type-only; the runtime is unchanged.

From `@baukit/data-contracts-expo-sqlite/testing`:

- `NodeSqliteDatabase` (implements `ExpoSqliteDatabase`) and `NodeSqliteConnection` (the statement
  methods, passed to exclusive-transaction tasks).
- `createSqliteMigrationConformanceTests(adapter): readonly SqliteMigrationConformanceTestCase[]`.
- `SqliteMigrationConformanceAdapter`, `SqliteMigrationConformanceTestCase`, `SqliteMigrationStep`.

## Supported runtimes

Node 24 or later, under Vitest or Jest 29 (through the `default` export condition). The entry point
imports `node:sqlite` and must not reach a React Native bundle.

## Failure behavior

SQLite errors reach the caller unchanged, including `database is locked`. A failed conformance case
throws an error starting `SQLite migration conformance failed:` that names the broken expectation.
Each case removes its temporary directory, whether it passes or fails.

## Privacy boundary

Test-only code on local temporary files. It sends nothing over the network, logs nothing, and puts
no row content in its own error messages.

## Breaks

None. The `ExpoSqliteStore` constructor parameter type widens; every Expo `SQLiteDatabase` still
fits.

## Product code a later adoption removes

- Eigenruhe: `mobile/src/db/contract-tests/better-sqlite.ts` and the three
  `as unknown as SQLiteDatabase` casts; `better-sqlite3` and `@types/better-sqlite3` if nothing else
  uses them.
- Redemut: `packages/data/src/node-sqlite.ts`, replaced by `ExpoSqliteDriver` over
  `NodeSqliteDatabase` in tests.
- Hebkit: `mobile/src/db/sqlite/adapters/better-sqlite.ts` once its tests run
  `ExpoSQLiteAdapter` over `NodeSqliteDatabase` (that needs the adapter to stop using
  `withTransactionAsync`), and the `better-sqlite3` dev dependencies.
- Tiefgang: `mobile/src/db/sqlite/adapters/better-sqlite.ts` after its runner becomes versioned and
  transactional, and the column-probing block in `migrate`.
