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

## Follow-up (2026-09-28)

Plan item F5 ships the queued raw-SQL connection that step 4 deferred.

### Source revisions

- Hebkit `841bf5d`: `mobile/src/db/sqlite/adapters/expo-sqlite.ts` (`ExpoSQLiteAdapter`,
  `SQLiteTransactionView`), `mobile/src/db/sqlite/adapter.ts` (`DatabaseAdapter`),
  `mobile/src/db/sqlite/migrations.ts:1364-1405` (six steps set `disableForeignKeys`).
- Redemut `a782538`: `packages/data/src/expo-sqlite.ts` (`ExpoSqliteDriver` and its local
  `ExpoSqliteDatabase`), `packages/data/src/driver.ts` (`SqliteDriver`),
  `packages/data/src/migrations.ts:298`.
- `expo-sqlite` 57.0.1 `src/SQLiteDatabase.ts`: `withExclusiveTransactionAsync` runs
  `transaction.execAsync('BEGIN')` before it calls the task; `withTransactionAsync` runs `BEGIN`
  on the root handle with no queue.
- Node 24.20.0 `node:sqlite` with SQLite 3.53.4.

### Decision: transactions on the supplied handle, not a second connection

Step 4 described the follow-up as root statements and exclusive transactions on one queue. That
cannot keep foreign keys on. SQLite ignores `PRAGMA foreign_keys` while a transaction is open, and
Expo's `withExclusiveTransactionAsync` has already run `BEGIN` on its new connection when the task
starts. A scratch run on `node:sqlite` confirmed it: after `BEGIN`, `PRAGMA foreign_keys = ON`
left `foreign_keys` at 0 and an orphan insert succeeded; with the pragma set before `BEGIN`, the
same insert failed with `FOREIGN KEY constraint failed`. Expo's open options have no foreign-key
switch, so a transaction connection always starts with them off.

So `ExpoSqliteConnection` runs `BEGIN IMMEDIATE`, the work, and `COMMIT` on the supplied handle,
and runs `PRAGMA foreign_keys = ON` on that handle before its first statement. The per-file queue
from item 1 is what keeps other callers out: every root statement, transaction, and `close()` of
the connection goes through `queueForFile(databasePath)`, which `ExpoSqliteStore` and the root
stores also use. A root statement issued while a transaction is open waits for its commit or
rollback, so it can no longer join it, which is Hebkit's `withTransactionAsync` defect. The
second-connection design would have made an unqueued statement fail with `database is locked`
instead of joining; with the handle design an unqueued statement on the same handle joins. Both are
outside the contract, and the README tells products to route raw statements through the connection.

The handle design also keeps Hebkit's six `disableForeignKeys` steps working: a root
`PRAGMA foreign_keys = OFF` before the transaction reaches the connection the transaction runs on.

Other choices:

- `BEGIN IMMEDIATE` takes the write lock at the start, so a conflict with an unqueued writer on
  another connection fails before the work runs.
- A failed commit (for example a deferred foreign key) rolls back and rejects with the commit
  error. When `ROLLBACK` itself fails because SQLite already rolled back, the original error wins.
- The transaction context's `transaction()` always rejects with a `TypeError`. Both product
  drivers join nested calls today (`SQLiteTransactionView.transaction` returns `work(this)`).
  A root call from inside the work still deadlocks, as it does for `withTransaction`; without
  async context the connection cannot tell it from a concurrent caller.
- SQLite errors pass through unchanged. The store's quota normalization does not apply, because
  a raw-SQL caller matches on SQLite messages.
- `get` returns `undefined` for no row, like the package's stores, instead of Expo's `null`.

### Shared code

`src/queued-database.ts` now holds what `ExpoSqliteStore` and the connection share:
`ExpoSqliteDatabase`, `fileScope`, `TransactionScope` (a statement scope that rejects with
`storage_closed` once finished, formerly inside `ExpoSqliteTransaction`), and `QueuedDatabase`
(enqueue on the file queue, `assertOpen`, one shared `close()` result, close only an owned
handle). `ExpoSqliteStore` uses them with unchanged behavior; its existing 97 package tests pass
unchanged.

### Public types

From `@baukit/data-contracts-expo-sqlite`:

- `ExpoSqliteConnection(database: ExpoSqliteDatabase, options?: ExpoSqliteConnectionOptions)`:
  `exec`, `run`, `get`, `all`, `transaction(work)`, `close()`.
- `ExpoSqliteConnectionOptions`: `closeDatabase?: boolean`, default false.
- `SqliteStatements`: `exec`, `run`, `get`, `all`, with `expo-sqlite`'s `SQLiteBindParams` and
  `SQLiteVariadicBindParams` overloads and `SQLiteRunResult`.
- `SqliteTransaction`: `SqliteStatements` plus a `transaction()` that always rejects.

### Tests

`src/connection.test.ts` runs on `NodeSqliteDatabase` (20 cases): binding, commit and result,
full rollback including DDL, rollback after a failed commit, foreign keys and `ON DELETE CASCADE`
inside a transaction, the pragma before the first statement, nested rejection, a settled context,
call order, queue release after a failure, both arrival orders against an `ExpoSqliteStore` on
the same file, close, and the five `createSqliteMigrationConformanceTests` cases through a history
runner on the connection. The concurrency case runs a root write during a transaction that rolls
back. It keeps the root row with the connection, and the same scenario on a `withTransactionAsync`
shaped driver loses it. Removing the queue from the connection fails four cases; skipping the
pragma fails three.

The device-conformance app gains four cases on the shared handle (root write during a rolled-back
transaction, foreign keys and cascade, DDL rollback and nested rejection, and ordering against an
`ExpoSqliteStore` in both arrival orders). They typecheck; they were not run here.

### Gates

Workspace `build`, `format:check`, `lint`, `test`, and `check` in `typescript/` pass, and
`tsc --noEmit` passes in `examples/expo-sqlite-conformance`.
`make expo-sqlite-conformance` was not run: another task held the Android emulator. Only that gate
confirms on real `expo-sqlite` that `BEGIN IMMEDIATE` through `execAsync` on the root handle holds
the transaction across queued statements, that `PRAGMA foreign_keys = ON` on the handle enforces
foreign keys and cascades inside it, and that the connection and `ExpoSqliteStore`'s exclusive
transactions on one file do not hit `database is locked`.

### Corrections to earlier text

Step 4 and the plan's adoption line tell Redemut to run `PRAGMA foreign_keys = ON` inside each
exclusive transaction, and say Hebkit would need the same. That pragma is a no-op there. Moving to
`ExpoSqliteConnection` is the fix.

### Breaks

None. One behavior to know: the first statement of an `ExpoSqliteConnection` turns foreign keys
on for its handle, so stores sharing that handle run with them on afterwards. The adapter's own
tables have no foreign keys.

### Product code a later adoption removes

- Hebkit: build `ExpoSQLiteAdapter` on `ExpoSqliteConnection` (`execute` to `run`, `query` to
  `all`), drop the `PRAGMA foreign_keys = ON` in `ExpoSQLiteAdapter.open`, and stop
  `SQLiteTransactionView.transaction` from joining. Then run `migrateDatabase` and the
  repository tests over `NodeSqliteDatabase` and retire `adapters/better-sqlite.ts`.
- Redemut: build `ExpoSqliteDriver` on `ExpoSqliteConnection`, delete its local
  `ExpoSqliteDatabase` interface and the `withExclusiveTransactionAsync` path, and drop the root
  `PRAGMA foreign_keys = ON` in `runMigrations`, which the connection now runs.

## Follow-up 0.5.1 (2026-09-29)

Hebkit's 0.5.0 adoption built `ExpoSQLiteAdapter` on `ExpoSqliteConnection` but could not do the
"stop `SQLiteTransactionView.transaction` from joining" step, because the connection's nested
`transaction()` always rejected. This follow-up makes it nest.

### Product evidence

- Hebkit `797fdff7`, `mobile/src/db/sqlite/adapters/expo-sqlite.ts:51-55`:
  `SQLiteTransactionView.transaction` returns `work(this)`, so nested work joins the enclosing
  transaction. A caught nested failure keeps whatever the nested work wrote before it failed.
- Hebkit composes repositories inside one transaction. `TransactionsRepository.run`
  (`db/sqlite/repositories.ts:2181-2195`) builds the settings, plans, nutrition-goal,
  questionnaire, and health-metric repositories on the transaction view, and their methods open
  their own transaction on it: `UserSettingsRepository.upsert` (`repositories.ts:2031`),
  `PlansRepository.create`, `setDefault`, `replaceDay`, `softDelete`, and `updatePlan`
  (`feature-repositories.ts:494,554,579,704,727`), and the nutrition goal `update`, `softDelete`,
  and `write` (`nutrition-repositories.ts:1584,1606,1623`). Callers are
  `features/onboarding/persistence.ts:81` and `features/nutrition/weight-adjustment-service.ts:185`,
  both with sequential awaits.
- `integrations/widget-snapshot.ts:446-475` wraps `transaction` on every adapter a repository is
  built on, including a transaction view, and refreshes the widget snapshot after the outermost
  call on that object settles. The refresh is awaited, so its reads finish before the caller's
  next nested call.
- Redemut `1d40e90`, `packages/data/src/expo-sqlite.ts:59-60`: `TransactionDriver.transaction()`
  forwards to the rejecting call and is typed `Promise<never>`. Redemut does not nest today.

### Decision: savepoints inside the queued transaction

`SqliteTransaction.transaction(work)` runs `SAVEPOINT baukit_nested_<depth>` on the same handle,
runs `work` with a new context, and runs `RELEASE` when it resolves. When `work` rejects it runs
`ROLLBACK TO` and `RELEASE` for that savepoint and rejects with the same error. If the caller
catches it, the enclosing transaction goes on without the nested writes or schema changes; if not,
the enclosing transaction rolls back as before. Released nested work still rolls back with a
failing enclosing level. A failed `RELEASE` is handled like a failed `COMMIT`.

Reasons:

- Joining, the old product behavior, keeps partial writes when a caller catches a nested failure.
  The migration conformance run with one savepoint per step shows the difference: with joining,
  "rolls back a failed step completely" fails because the failed step's table and column stay.
- Rejecting, the 0.5.0 behavior, forced Hebkit to keep its own joining view and blocked the
  adoption step.
- Savepoints are plain SQLite, run on the handle the root transaction already holds, and work
  inside `BEGIN IMMEDIATE`, so nothing about the queue or the pragma order changes.

Invariants kept:

- Per-file queue. A nested call never enters the queue. It runs on the transaction's handle while
  the enclosing root transaction holds its queue turn, so it cannot deadlock behind itself, and a
  root call another caller queued during the transaction still runs after the commit.
- Foreign keys. The pragma still runs once on the handle before the first root statement, before
  any `BEGIN IMMEDIATE`. Savepoints inherit it; a node test and the device case check an orphan
  insert inside a savepoint fails.
- Interleaving. SQLite savepoints are a stack, and `RELEASE` of an outer name releases every newer
  one. Two open siblings would corrupt each other, so only the innermost open level accepts work:
  while a nested transaction is open, statements and `transaction()` on any enclosing context
  reject with a `TypeError`. The flag is set before the first `await`, so the rejection is
  deterministic. I chose rejection over queueing siblings because a queue would deadlock when a
  nested callback calls the enclosing context it captured, which rejection reports at once.
- A root call on the connection from inside `work` still deadlocks, as before. Without async
  context the connection cannot tell it from a concurrent caller.

`ExpoSqliteStore.withTransaction` keeps joining nested calls, as the shared storage contract in
`@baukit/data-contracts` requires. Only the raw connection nests.

### Tests

Node (`connection.test.ts`, on `NodeSqliteDatabase`): ten nesting cases (commit with result,
caught failure with DDL rollback, uncaught failure, enclosing failure after release, three levels
with siblings, foreign keys, enclosing work rejected while nested is open, settled nested context,
no wait on a queue another caller is on, and one helper used at the root and nested), plus the
five migration conformance cases over a runner that applies each pending step in its own
savepoint inside one transaction, and a case that keeps the steps before a failed one. The
package runs 132 tests. Replacing savepoints with joining fails six cases, one of them the
conformance suite's partially applied migration check.

Device (`examples/expo-sqlite-conformance`): the case that checked nested rejection now checks
DDL rollback only, and a new case runs nested transactions on real `expo-sqlite`: a caught nested
failure with an `ALTER TABLE`, a released savepoint inside a nested level, an enclosing statement
rejected while a nested level is open, a root write from another caller issued during the
transaction, and an enclosing rollback that discards released nested work.

### Gates

Workspace `build`, `format:check`, `lint`, `test`, and `check` in `typescript/` pass (132 tests
in this package). `tsc --noEmit` passes in `examples/expo-sqlite-conformance`.
`make expo-sqlite-conformance` with `METRO_PORT=8082 CI=1` under the emulator lock passed 31 cases
on real `expo-sqlite` (30 before plus the new savepoint case).

### Breaks

`SqliteTransaction.transaction` changes from `(work) => Promise<never>` to
`<TResult>(work: (transaction: SqliteTransaction) => Promise<TResult> | TResult) =>
Promise<TResult>`, and a nested call now runs instead of rejecting. Redemut's
`TransactionDriver.transaction(): Promise<never>` stops type-checking against the new signature.

### Product code a later adoption removes

- Hebkit: in `mobile/src/db/sqlite/adapters/expo-sqlite.ts`, make `SQLiteTransactionView` hold
  its `SqliteTransaction` and implement `transaction(work)` as
  `this.open.transaction((nested) => work(new SQLiteTransactionView(nested)))`, deleting the
  joining `work(this)` and its comment. Nothing in the call sites above changes. The Dexie path
  (`db/dexie/*.web.ts`) is separate and untouched.
- Redemut: in `packages/data/src/expo-sqlite.ts`, make `TransactionDriver.transaction` forward
  `operation` through `this.#open.transaction((open) => operation(new TransactionDriver(open)))`
  instead of the `Promise<never>` stub.
