import {
  InMemoryScopedPersistenceRegistryStore,
  MAX_PAGE_SIZE,
  ScopedPersistenceLifecycle,
  type JsonValue,
  type StoredRecord,
  recheckServerSubjectBeforeSyncAdoption,
} from "@baukit/data-contracts";
import {
  ExpoSqliteConnection,
  ExpoSqliteStore,
} from "@baukit/data-contracts-expo-sqlite";
import * as Crypto from "expo-crypto";
import * as SQLite from "expo-sqlite";

interface ContractRecord extends StoredRecord {
  readonly label: string;
  readonly payload: JsonValue;
}

interface Case {
  readonly name: string;
  readonly run: () => Promise<void>;
}

const RECORDS = {
  first: { id: "b", label: "second", payload: { position: 2 } },
  before: { id: "a", label: "inserted before cursor", payload: null },
  second: { id: "c", label: "third", payload: [3] },
  third: { id: "d", label: "fourth", payload: true },
} as const satisfies Record<string, ContractRecord>;

const DATABASE_NAME = "baukit-contract.db";
let namespaceSequence = 0;

interface IdentityPersistence {
  readonly store: ExpoSqliteStore<ContractRecord>;
  close(): Promise<void>;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertDeep(actual: unknown, expected: unknown, message: string): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  assert(
    actualJson === expectedJson,
    `${message}: expected ${expectedJson}, received ${actualJson}`,
  );
}

async function expectReject(
  operation: Promise<unknown>,
  message: string,
): Promise<unknown> {
  try {
    await operation;
  } catch (cause) {
    return cause;
  }
  throw new Error(`${message}: promise resolved`);
}

function errorCode(cause: unknown): unknown {
  return typeof cause === "object" && cause !== null
    ? Reflect.get(cause, "code")
    : undefined;
}

function syntheticQuotaError(): Error {
  const error = new Error("simulated adapter quota");
  error.name = "QuotaExceededError";
  return error;
}

const TRANSACTION_HOLD_MS = 50;
const ROOT_WRITES_BEFORE_TRANSACTION = 20;

function hold(milliseconds: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, milliseconds);
  });
}

function identityDigest(value: string): Promise<string> {
  return Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, value);
}

async function deleteIdentityDatabases(
  names: ReadonlySet<string>,
): Promise<void> {
  await Promise.all(
    [...names].map((name) =>
      SQLite.deleteDatabaseAsync(`${name}.db`).catch(() => undefined),
    ),
  );
}

export async function runConformance(): Promise<{ readonly passed: number }> {
  await SQLite.deleteDatabaseAsync(DATABASE_NAME).catch(() => undefined);
  const database = await SQLite.openDatabaseAsync(DATABASE_NAME);

  const makeStore = async (): Promise<ExpoSqliteStore<ContractRecord>> => {
    namespaceSequence += 1;
    const store = new ExpoSqliteStore<ContractRecord>(
      database,
      `contract-${namespaceSequence}`,
    );
    await store.initialize();
    return store;
  };

  // These cases mirror every case registered by @baukit/data-contracts/vitest.
  // The small runner avoids bringing Vitest's Node runtime into a native app.
  const cases: Case[] = [
    {
      name: "key/value JSON round-trip and reference isolation",
      run: async () => {
        const store = await makeStore();
        const value: JsonValue = {
          array: [null, true, 42, "text"],
          nested: { ready: false },
        };
        await store.keyValues.set("value", value);
        assertDeep(
          await store.keyValues.get("value"),
          value,
          "JSON value did not round-trip",
        );
        const loaded = (await store.keyValues.get("value")) as {
          nested: { ready: boolean };
        };
        loaded.nested.ready = true;
        assertDeep(
          await store.keyValues.get("value"),
          value,
          "loaded value leaked a mutable reference",
        );
      },
    },
    {
      name: "missing key/value operations",
      run: async () => {
        const store = await makeStore();
        assert(
          (await store.keyValues.get("missing")) === undefined,
          "missing key was present",
        );
        await store.keyValues.delete("missing");
      },
    },
    {
      name: "key/value replacement, delete, and clear",
      run: async () => {
        const store = await makeStore();
        await store.keyValues.set("first", 1);
        await store.keyValues.set("first", 2);
        await store.keyValues.set("second", 3);
        assert(
          (await store.keyValues.get("first")) === 2,
          "replacement failed",
        );
        await store.keyValues.delete("first");
        assert(
          (await store.keyValues.get("first")) === undefined,
          "delete failed",
        );
        await store.keyValues.clear();
        assert(
          (await store.keyValues.get("second")) === undefined,
          "clear failed",
        );
      },
    },
    {
      name: "record CRUD and replacement",
      run: async () => {
        const store = await makeStore();
        await store.records.put(RECORDS.first);
        assertDeep(
          await store.records.get("b"),
          RECORDS.first,
          "record put failed",
        );
        const replacement = { ...RECORDS.first, label: "replacement" };
        await store.records.put(replacement);
        assertDeep(
          await store.records.get("b"),
          replacement,
          "record replacement failed",
        );
        await store.records.delete("b");
        assert(
          (await store.records.get("b")) === undefined,
          "record delete failed",
        );
        await store.records.delete("missing");
      },
    },
    {
      name: "empty terminal record page",
      run: async () => {
        const store = await makeStore();
        assertDeep(
          await store.records.list({ limit: 2 }),
          { items: [], nextCursor: null },
          "empty page mismatch",
        );
      },
    },
    {
      name: "exact-size terminal record page",
      run: async () => {
        const store = await makeStore();
        await store.records.put(RECORDS.second);
        await store.records.put(RECORDS.first);
        assertDeep(
          await store.records.list({ limit: 2 }),
          { items: [RECORDS.first, RECORDS.second], nextCursor: null },
          "terminal page mismatch",
        );
      },
    },
    {
      name: "stable keyset record cursor",
      run: async () => {
        const store = await makeStore();
        await store.records.put(RECORDS.first);
        await store.records.put(RECORDS.second);
        await store.records.put(RECORDS.third);
        const page = await store.records.list({ limit: 1 });
        assertDeep(page.items, [RECORDS.first], "first page mismatch");
        assert(page.nextCursor !== null, "first page did not return a cursor");
        await store.records.put(RECORDS.before);
        assertDeep(
          await store.records.list({ cursor: page.nextCursor, limit: 2 }),
          { items: [RECORDS.second, RECORDS.third], nextCursor: null },
          "keyset cursor was unstable",
        );
      },
    },
    {
      name: "invalid record bounds and cursors",
      run: async () => {
        const store = await makeStore();
        await expectReject(store.records.list({ limit: 0 }), "zero limit");
        await expectReject(
          store.records.list({ limit: MAX_PAGE_SIZE + 1 }),
          "unbounded limit",
        );
        await expectReject(
          store.records.list({ limit: 1.5 }),
          "fractional limit",
        );
        await expectReject(
          store.records.list({ cursor: "not-an-adapter-cursor" }),
          "invalid cursor",
        );
      },
    },
    {
      name: "schema metadata replacement",
      run: async () => {
        const store = await makeStore();
        assert(
          (await store.schemaMetadata.getSchemaMeta()) === undefined,
          "schema metadata existed",
        );
        await store.schemaMetadata.setSchemaMeta({ name: "notes", version: 1 });
        assertDeep(
          await store.schemaMetadata.getSchemaMeta(),
          { name: "notes", version: 1 },
          "schema metadata mismatch",
        );
        await store.schemaMetadata.setSchemaMeta({ name: "notes", version: 2 });
        assertDeep(
          await store.schemaMetadata.getSchemaMeta(),
          { name: "notes", version: 2 },
          "schema upgrade mismatch",
        );
      },
    },
    {
      name: "compound transaction result and commit",
      run: async () => {
        const store = await makeStore();
        const result = await store.withTransaction(async (transaction) => {
          await transaction.keyValues.set("first", 1);
          await transaction.keyValues.set("second", 2);
          await transaction.records.put(RECORDS.first);
          await transaction.records.put(RECORDS.second);
          await transaction.schemaMetadata.setSchemaMeta({
            name: "contract",
            version: 1,
          });
          return "committed";
        });
        assert(result === "committed", "transaction result was lost");
        assert(
          (await store.keyValues.get("first")) === 1,
          "first transaction write missing",
        );
        assertDeep(
          (await store.records.list()).items,
          [RECORDS.first, RECORDS.second],
          "record writes missing",
        );
      },
    },
    {
      name: "compound transaction rollback",
      run: async () => {
        const store = await makeStore();
        await store.keyValues.set("preserved", "before");
        await expectReject(
          store.withTransaction(async (transaction) => {
            await transaction.keyValues.set("preserved", "after");
            await transaction.records.put(RECORDS.first);
            await transaction.schemaMetadata.setSchemaMeta({
              name: "contract",
              version: 1,
            });
            throw new Error("deliberate rollback");
          }),
          "rollback transaction",
        );
        assert(
          (await store.keyValues.get("preserved")) === "before",
          "rollback replaced preserved value",
        );
        assert(
          (await store.records.get("b")) === undefined,
          "rollback retained record",
        );
      },
    },
    {
      name: "record and outbox-shaped transaction",
      run: async () => {
        const store = await makeStore();
        await store.withTransaction(async (transaction) => {
          await transaction.records.put(RECORDS.first);
          await transaction.keyValues.set("outbox:mutation-1", {
            entityId: "b",
            operation: "put",
          });
        });
        assertDeep(
          await store.records.get("b"),
          RECORDS.first,
          "atomic record missing",
        );
        assertDeep(
          await store.keyValues.get("outbox:mutation-1"),
          { entityId: "b", operation: "put" },
          "atomic outbox entry missing",
        );
      },
    },
    {
      name: "nested transaction join",
      run: async () => {
        const store = await makeStore();
        const result = await store.withTransaction(async (transaction) => {
          await transaction.records.put(RECORDS.first);
          return transaction.withTransaction(async (nested) => {
            assert(nested === transaction, "nested transaction did not join");
            await nested.keyValues.set("nested", true);
            return "nested-result";
          });
        });
        assert(result === "nested-result", "nested result was lost");
        assert(
          (await store.keyValues.get("nested")) === true,
          "nested write missing",
        );
      },
    },
    {
      name: "outer rollback includes nested writes",
      run: async () => {
        const store = await makeStore();
        await expectReject(
          store.withTransaction(async (transaction) => {
            await transaction.withTransaction(async (nested) => {
              await nested.records.put(RECORDS.first);
              await nested.keyValues.set("outbox:mutation-1", true);
            });
            throw new Error("outer failure");
          }),
          "outer rollback",
        );
        assert(
          (await store.records.get("b")) === undefined,
          "nested record survived rollback",
        );
        assert(
          (await store.keyValues.get("outbox:mutation-1")) === undefined,
          "nested key survived rollback",
        );
      },
    },
    {
      name: "quota normalization and rollback",
      run: async () => {
        const store = await makeStore();
        const cause = await expectReject(
          store.withTransaction(async (transaction) => {
            await transaction.records.put(RECORDS.first);
            throw syntheticQuotaError();
          }),
          "quota failure",
        );
        assert(
          errorCode(cause) === "storage_quota_exceeded",
          "quota error code was not normalized",
        );
        assert(
          (await store.records.get("b")) === undefined,
          "quota failure did not roll back",
        );
      },
    },
    {
      name: "closed adapter errors",
      run: async () => {
        const store = await makeStore();
        await store.close();
        const operations = [
          store.keyValues.get("closed"),
          store.records.put(RECORDS.first),
          store.schemaMetadata.getSchemaMeta(),
          store.withTransaction(() => undefined),
        ];
        for (const operation of operations) {
          const cause = await expectReject(operation, "operation after close");
          assert(
            errorCode(cause) === "storage_closed",
            "closed error code mismatch",
          );
        }
      },
    },
    {
      name: "concurrent root transaction serialization",
      run: async () => {
        const store = await makeStore();
        const events: string[] = [];
        const first = store.withTransaction(async (transaction) => {
          events.push("first:start");
          await transaction.keyValues.set("order", "first");
          events.push("first:end");
        });
        const second = store.withTransaction(async (transaction) => {
          events.push("second:start");
          assert(
            (await transaction.keyValues.get("order")) === "first",
            "second transaction started early",
          );
          await transaction.keyValues.set("order", "second");
          events.push("second:end");
        });
        await Promise.all([first, second]);
        assertDeep(
          events,
          ["first:start", "first:end", "second:start", "second:end"],
          "transaction order mismatch",
        );
      },
    },
    {
      name: "real SQLite root operations called before a transaction finish first",
      run: async () => {
        const store = await makeStore();
        const rootWrites = Array.from(
          { length: ROOT_WRITES_BEFORE_TRANSACTION },
          (_, index) =>
            store.records.put({
              id: `root-${String(index).padStart(2, "0")}`,
              label: "root",
              payload: index,
            }),
        );
        const rootKeyValue = store.keyValues.set("root", "before");
        const rootSchema = store.schemaMetadata.setSchemaMeta({
          name: "root",
          version: 1,
        });
        const observed = store.withTransaction(async (transaction) => {
          const page = await transaction.records.list({
            limit: ROOT_WRITES_BEFORE_TRANSACTION,
          });
          await transaction.keyValues.set("transaction", "after");
          return {
            records: page.items.length,
            keyValue: await transaction.keyValues.get("root"),
            schema: await transaction.schemaMetadata.getSchemaMeta(),
          };
        });
        await Promise.all([...rootWrites, rootKeyValue, rootSchema]);
        assertDeep(
          await observed,
          {
            records: ROOT_WRITES_BEFORE_TRANSACTION,
            keyValue: "before",
            schema: { name: "root", version: 1 },
          },
          "transaction overlapped earlier root operations",
        );
      },
    },
    {
      name: "real SQLite root operations called during a transaction wait for its commit",
      run: async () => {
        const store = await makeStore();
        let entered: () => void = () => undefined;
        const transactionEntered = new Promise<void>((resolve) => {
          entered = resolve;
        });
        const transaction = store.withTransaction(async (context) => {
          await context.records.put(RECORDS.first);
          await context.keyValues.set("state", "committed");
          await context.schemaMetadata.setSchemaMeta({
            name: "transaction",
            version: 2,
          });
          entered();
          await hold(TRANSACTION_HOLD_MS);
        });
        await transactionEntered;
        const [record, state, schema] = await Promise.all([
          store.records.get(RECORDS.first.id),
          store.keyValues.get("state"),
          store.schemaMetadata.getSchemaMeta(),
          store.records.put(RECORDS.second),
          store.keyValues.set("root", "after"),
        ]);
        await transaction;
        assertDeep(
          { record, state, schema },
          {
            record: RECORDS.first,
            state: "committed",
            schema: { name: "transaction", version: 2 },
          },
          "root read overlapped an open transaction",
        );
        assertDeep(
          await store.records.get(RECORDS.second.id),
          RECORDS.second,
          "root write after the transaction was lost",
        );
      },
    },
    {
      name: "real SQLite stores sharing one handle do not overlap",
      run: async () => {
        const first = await makeStore();
        const second = await makeStore();
        let entered: () => void = () => undefined;
        const transactionEntered = new Promise<void>((resolve) => {
          entered = resolve;
        });
        const transaction = first.withTransaction(async (context) => {
          await context.records.put(RECORDS.first);
          entered();
          await hold(TRANSACTION_HOLD_MS);
        });
        await transactionEntered;
        await Promise.all([
          second.records.put(RECORDS.second),
          second.keyValues.set("neighbor", true),
        ]);
        await transaction;
        assertDeep(
          await first.records.get(RECORDS.first.id),
          RECORDS.first,
          "first namespace lost its transaction write",
        );
        assertDeep(
          await second.records.get(RECORDS.second.id),
          RECORDS.second,
          "second namespace lost its root write",
        );
      },
    },
    {
      name: "real SQLite raw connection keeps root statements out of another caller's transaction",
      run: async () => {
        const connection = new ExpoSqliteConnection(database);
        await connection.exec("CREATE TABLE raw_events (label TEXT NOT NULL)");
        let entered: () => void = () => undefined;
        const transactionEntered = new Promise<void>((resolve) => {
          entered = resolve;
        });
        const transaction = connection.transaction(async (context) => {
          await context.run(
            "INSERT INTO raw_events (label) VALUES (?)",
            "transaction",
          );
          entered();
          await hold(TRANSACTION_HOLD_MS);
          throw new Error("roll back");
        });
        await transactionEntered;
        const root = connection.run(
          "INSERT INTO raw_events (label) VALUES (?)",
          "root",
        );
        await expectReject(transaction, "rolled-back transaction");
        await root;
        assertDeep(
          await connection.all("SELECT label FROM raw_events ORDER BY rowid"),
          [{ label: "root" }],
          "root write joined another caller's transaction",
        );
      },
    },
    {
      name: "real SQLite raw connection enforces foreign keys inside transactions",
      run: async () => {
        const connection = new ExpoSqliteConnection(database);
        await connection.exec(`CREATE TABLE raw_parents (id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE raw_children (
  id TEXT PRIMARY KEY NOT NULL,
  parent_id TEXT NOT NULL REFERENCES raw_parents (id) ON DELETE CASCADE
);`);
        await expectReject(
          connection.transaction((context) =>
            context.run(
              "INSERT INTO raw_children (id, parent_id) VALUES (?, ?)",
              "orphan",
              "missing",
            ),
          ),
          "orphan insert inside a transaction",
        );
        await connection.transaction(async (context) => {
          await context.run(
            "INSERT INTO raw_parents (id) VALUES (?)",
            "parent",
          );
          await context.run(
            "INSERT INTO raw_children (id, parent_id) VALUES (?, ?)",
            "child",
            "parent",
          );
          await context.run("DELETE FROM raw_parents WHERE id = ?", "parent");
        });
        assertDeep(
          await connection.all("SELECT id FROM raw_children"),
          [],
          "ON DELETE CASCADE did not run inside a transaction",
        );
      },
    },
    {
      name: "real SQLite raw connection rolls back schema changes",
      run: async () => {
        const connection = new ExpoSqliteConnection(database);
        await connection.exec("CREATE TABLE raw_steps (label TEXT NOT NULL)");
        await expectReject(
          connection.transaction(async (context) => {
            await context.exec(
              "ALTER TABLE raw_steps ADD COLUMN rank INTEGER NOT NULL DEFAULT 0; CREATE TABLE raw_partial (id TEXT);",
            );
            await context.exec("INSERT INTO raw_missing (id) VALUES ('fails')");
          }),
          "failing migration step",
        );
        assertDeep(
          await connection.all(
            "SELECT name FROM pragma_table_info('raw_steps') ORDER BY cid",
          ),
          [{ name: "label" }],
          "a failed step left a column behind",
        );
        assert(
          (await connection.get(
            "SELECT name FROM sqlite_master WHERE name = 'raw_partial'",
          )) === undefined,
          "a failed step left a table behind",
        );
      },
    },
    {
      name: "real SQLite raw connection nests transactions as savepoints",
      run: async () => {
        const connection = new ExpoSqliteConnection(database);
        await connection.exec("CREATE TABLE raw_nested (label TEXT NOT NULL)");
        const insert = "INSERT INTO raw_nested (label) VALUES (?)";
        const rows = () =>
          connection.all("SELECT label FROM raw_nested ORDER BY rowid");
        let root: Promise<unknown> = Promise.resolve();
        await connection.transaction(async (context) => {
          await context.run(insert, "outer");
          root = connection.run(insert, "other caller");
          await expectReject(
            context.transaction(async (nested) => {
              await nested.run(insert, "rolled back");
              await nested.exec(
                "ALTER TABLE raw_nested ADD COLUMN ghost TEXT; CREATE TABLE raw_nested_partial (id TEXT);",
              );
              throw new Error("inner failed");
            }),
            "failing nested transaction",
          );
          let entered: () => void = () => undefined;
          const nestedEntered = new Promise<void>((resolve) => {
            entered = resolve;
          });
          const kept = context.transaction(async (nested) => {
            await nested.transaction((inner) => inner.run(insert, "released"));
            entered();
            await hold(TRANSACTION_HOLD_MS);
          });
          await nestedEntered;
          await expectReject(
            context.run(insert, "outer while nested open"),
            "outer statement while a nested transaction is open",
          );
          await kept;
        });
        await root;
        assertDeep(
          await rows(),
          [{ label: "outer" }, { label: "released" }, { label: "other caller" }],
          "nested transactions did not commit or roll back as savepoints",
        );
        assertDeep(
          await connection.all(
            "SELECT name FROM pragma_table_info('raw_nested') ORDER BY cid",
          ),
          [{ name: "label" }],
          "a rolled-back nested transaction left a column behind",
        );
        await expectReject(
          connection.transaction(async (context) => {
            await context.transaction((nested) =>
              nested.run(insert, "released then rolled back"),
            );
            throw new Error("outer failed");
          }),
          "failing outer transaction",
        );
        assertDeep(
          (await rows()).length,
          3,
          "an outer rollback kept released nested work",
        );
      },
    },
    {
      name: "real SQLite raw connection and ExpoSqliteStore share one queue",
      run: async () => {
        const store = await makeStore();
        const connection = new ExpoSqliteConnection(database);
        await connection.exec("CREATE TABLE raw_shared (label TEXT NOT NULL)");
        let storeEntered: () => void = () => undefined;
        const storeHeld = new Promise<void>((resolve) => {
          storeEntered = resolve;
        });
        const storeTransaction = store.withTransaction(async (context) => {
          await context.records.put(RECORDS.first);
          storeEntered();
          await hold(TRANSACTION_HOLD_MS);
        });
        await storeHeld;
        await Promise.all([
          connection.run(
            "INSERT INTO raw_shared (label) VALUES (?)",
            "during store transaction",
          ),
          storeTransaction,
        ]);
        let rawEntered: () => void = () => undefined;
        const rawHeld = new Promise<void>((resolve) => {
          rawEntered = resolve;
        });
        const rawTransaction = connection.transaction(async (context) => {
          await context.run(
            "INSERT INTO raw_shared (label) VALUES (?)",
            "rolled back",
          );
          rawEntered();
          await hold(TRANSACTION_HOLD_MS);
          throw new Error("roll back");
        });
        await rawHeld;
        const storeWrite = store.records.put(RECORDS.second);
        await expectReject(rawTransaction, "rolled-back raw transaction");
        await storeWrite;
        assertDeep(
          await connection.all("SELECT label FROM raw_shared ORDER BY rowid"),
          [{ label: "during store transaction" }],
          "raw statements overlapped the store transaction",
        );
        assertDeep(
          await store.records.get(RECORDS.second.id),
          RECORDS.second,
          "store write joined the rolled-back raw transaction",
        );
      },
    },
    {
      name: "real SQLite namespace isolation",
      run: async () => {
        const first = new ExpoSqliteStore<ContractRecord>(
          database,
          "native-first",
        );
        const second = new ExpoSqliteStore<ContractRecord>(
          database,
          "native-second",
        );
        await first.initialize();
        await second.initialize();
        await first.records.put({ id: "same", label: "first", payload: 1 });
        await second.records.put({ id: "same", label: "second", payload: 2 });
        assertDeep(
          await first.records.get("same"),
          { id: "same", label: "first", payload: 1 },
          "first namespace collided",
        );
        assertDeep(
          await second.records.get("same"),
          { id: "same", label: "second", payload: 2 },
          "second namespace collided",
        );
      },
    },
    {
      name: "malformed persisted record is redacted",
      run: async () => {
        const store = new ExpoSqliteStore<ContractRecord>(
          database,
          "native-private",
        );
        await store.initialize();
        await database.runAsync(
          "INSERT INTO baukit_records (namespace, id, payload) VALUES (?, ?, ?)",
          "native-private",
          "record",
          "private journal content {",
        );
        const cause = await expectReject(
          store.records.get("record"),
          "malformed payload",
        );
        const message = cause instanceof Error ? cause.message : String(cause);
        assert(
          message === "The local database contains an invalid record.",
          "malformed error was unstable",
        );
        assert(!message.includes("journal"), "malformed error leaked payload");
      },
    },
    {
      name: "real SQLite offline E to F to E identity isolation",
      run: async () => {
        const databaseNames = new Set<string>();
        const events: string[] = [];
        let resetCount = 0;
        const lifecycle = new ScopedPersistenceLifecycle<IdentityPersistence>({
          namespace: "expo-conformance",
          registry: new InMemoryScopedPersistenceRegistryStore(),
          digest: identityDigest,
          open: async ({ storeName, subject }) => {
            databaseNames.add(storeName);
            events.push(`open:${subject}`);
            const connection = await SQLite.openDatabaseAsync(
              `${storeName}.db`,
            );
            const store = new ExpoSqliteStore<ContractRecord>(
              connection,
              "identity",
              { closeDatabase: true },
            );
            await store.initialize();
            return {
              store,
              close: async () => {
                events.push(`close:start:${subject}`);
                await store.close();
                events.push(`close:end:${subject}`);
              },
            };
          },
          resetUserScopedState: () => {
            resetCount += 1;
          },
        });
        try {
          const accountE = await lifecycle.selectSubject("account-e");
          assert(accountE !== undefined, "account E partition was not ready");
          await accountE.persistence.store.withTransaction(
            async (transaction) => {
              await transaction.records.put({
                id: "shared",
                label: "account E",
                payload: 1,
              });
              await transaction.keyValues.set("outbox:pending", "mutation-e");
            },
          );
          const accountF = await lifecycle.selectSubject("account-f");
          assert(accountF !== undefined, "account F partition was not ready");
          assert(
            (await accountF.persistence.store.records.get("shared")) ===
              undefined,
            "account F read account E data",
          );
          assert(
            (await accountF.persistence.store.keyValues.get(
              "outbox:pending",
            )) === undefined,
            "account F read account E outbox",
          );
          await accountF.persistence.store.records.put({
            id: "shared",
            label: "account F",
            payload: 2,
          });
          const accountEReturned = await lifecycle.selectSubject("account-e");
          assert(
            accountEReturned !== undefined,
            "account E return partition was not ready",
          );
          assertDeep(
            await accountEReturned.persistence.store.records.get("shared"),
            { id: "shared", label: "account E", payload: 1 },
            "account E data changed across account F",
          );
          assert(
            (await accountEReturned.persistence.store.keyValues.get(
              "outbox:pending",
            )) === "mutation-e",
            "account E outbox changed across account F",
          );
          assert(
            events.indexOf("close:end:account-e") <
              events.indexOf("open:account-f"),
            "account F opened before account E closed",
          );
          assert(resetCount >= 3, "user-scoped memory was not reset");
          await lifecycle.clear();
        } finally {
          await deleteIdentityDatabases(databaseNames);
        }
      },
    },
    {
      name: "real SQLite legacy claim and corrupt-registry blocking",
      run: async () => {
        const legacyName = "baukit-identity-legacy";
        await SQLite.deleteDatabaseAsync(`${legacyName}.db`).catch(
          () => undefined,
        );
        const legacyConnection = await SQLite.openDatabaseAsync(
          `${legacyName}.db`,
        );
        const legacy = new ExpoSqliteStore<ContractRecord>(
          legacyConnection,
          "identity",
          { closeDatabase: true },
        );
        await legacy.initialize();
        await legacy.records.put({
          id: "legacy",
          label: "legacy account E",
          payload: null,
        });
        await legacy.close();
        const databaseNames = new Set<string>([legacyName]);
        const lifecycle = new ScopedPersistenceLifecycle<
          ExpoSqliteStore<ContractRecord>
        >({
          namespace: "expo-legacy-conformance",
          registry: new InMemoryScopedPersistenceRegistryStore(),
          digest: identityDigest,
          legacyStoreName: legacyName,
          inspectLegacy: () =>
            Promise.resolve({ exists: true, ownership: "claimable" }),
          open: async ({ storeName }) => {
            databaseNames.add(storeName);
            const connection = await SQLite.openDatabaseAsync(
              `${storeName}.db`,
            );
            const store = new ExpoSqliteStore<ContractRecord>(
              connection,
              "identity",
              { closeDatabase: true },
            );
            await store.initialize();
            return store;
          },
          resetUserScopedState: () => undefined,
        });
        try {
          const accountE = await lifecycle.selectSubject("account-e");
          assert(
            accountE?.storeName === legacyName,
            "claimable legacy store was not selected",
          );
          assert(
            (await accountE.persistence.records.get("legacy")) !== undefined,
            "legacy record was not preserved",
          );
          const accountF = await lifecycle.selectSubject("account-f");
          assert(accountF !== undefined, "account F partition was not ready");
          assert(
            accountF.storeName !== legacyName,
            "legacy store was claimed more than once",
          );
          assert(
            (await accountF.persistence.records.get("legacy")) === undefined,
            "second account read the legacy partition",
          );
          await lifecycle.clear();

          let opens = 0;
          const corrupt = new ScopedPersistenceLifecycle<
            ExpoSqliteStore<ContractRecord>
          >({
            namespace: "expo-corrupt-conformance",
            registry: new InMemoryScopedPersistenceRegistryStore("{broken"),
            digest: identityDigest,
            open: async ({ storeName }) => {
              opens += 1;
              const connection = await SQLite.openDatabaseAsync(
                `${storeName}.db`,
              );
              const store = new ExpoSqliteStore<ContractRecord>(
                connection,
                "identity",
                { closeDatabase: true },
              );
              await store.initialize();
              return store;
            },
            resetUserScopedState: () => undefined,
          });
          const cause = await expectReject(
            corrupt.selectSubject("account-e"),
            "corrupt identity registry",
          );
          assert(
            errorCode(cause) === "persistence_identity_mismatch",
            "corrupt registry error code mismatch",
          );
          assert(opens === 0, "corrupt registry opened a domain database");
        } finally {
          await deleteIdentityDatabases(databaseNames);
        }
      },
    },
    {
      name: "real SQLite session expiry and server-subject mismatch block",
      run: async () => {
        const databaseNames = new Set<string>();
        let adopted = false;
        const lifecycle = new ScopedPersistenceLifecycle<
          ExpoSqliteStore<ContractRecord>
        >({
          namespace: "expo-expiry-conformance",
          registry: new InMemoryScopedPersistenceRegistryStore(),
          digest: identityDigest,
          open: async ({ storeName }) => {
            databaseNames.add(storeName);
            const connection = await SQLite.openDatabaseAsync(
              `${storeName}.db`,
            );
            const store = new ExpoSqliteStore<ContractRecord>(
              connection,
              "identity",
              { closeDatabase: true },
            );
            await store.initialize();
            return store;
          },
          resetUserScopedState: () => undefined,
        });
        try {
          const accountE = await lifecycle.selectSubject("account-e");
          assert(accountE !== undefined, "account E partition was not ready");
          await accountE.persistence.records.put({
            id: "preserved",
            label: "before sync",
            payload: true,
          });
          const cause = await expectReject(
            recheckServerSubjectBeforeSyncAdoption({
              partitionSubject: "account-e",
              readServerSubject: () => Promise.resolve("account-f"),
              adopt: () => {
                adopted = true;
              },
            }),
            "server subject mismatch",
          );
          assert(
            errorCode(cause) === "persistence_identity_mismatch",
            "server mismatch error code mismatch",
          );
          assert(!adopted, "sync adoption ran after a subject mismatch");
          assertDeep(
            await accountE.persistence.records.get("preserved"),
            { id: "preserved", label: "before sync", payload: true },
            "server mismatch changed local data",
          );
          await lifecycle.handleSessionExpired();
          assert(
            lifecycle.state.status === "blocked" &&
              lifecycle.state.reason === "session-expired",
            "terminal expiry did not block persistence",
          );
          assert(
            lifecycle.current() === undefined,
            "expired persistence remained available",
          );
        } finally {
          await deleteIdentityDatabases(databaseNames);
        }
      },
    },
  ];

  try {
    for (const testCase of cases) {
      try {
        await testCase.run();
      } catch (cause) {
        const detail = cause instanceof Error ? cause.message : String(cause);
        throw new Error(`${testCase.name}: ${detail}`);
      }
    }
  } finally {
    await database.closeAsync();
  }

  const reopenName = "baukit-reopen.db";
  await SQLite.deleteDatabaseAsync(reopenName).catch(() => undefined);
  const firstConnection = await SQLite.openDatabaseAsync(reopenName);
  const firstStore = new ExpoSqliteStore<ContractRecord>(
    firstConnection,
    "upgrade",
  );
  await firstStore.initialize();
  await firstStore.records.put(RECORDS.first);
  await firstStore.schemaMetadata.setSchemaMeta({
    name: "contract",
    version: 1,
  });
  await firstConnection.closeAsync();
  const reopenedConnection = await SQLite.openDatabaseAsync(reopenName);
  const reopenedStore = new ExpoSqliteStore<ContractRecord>(
    reopenedConnection,
    "upgrade",
  );
  await reopenedStore.initialize();
  assertDeep(
    await reopenedStore.records.get("b"),
    RECORDS.first,
    "reopened database lost its record",
  );
  assertDeep(
    await reopenedStore.schemaMetadata.getSchemaMeta(),
    { name: "contract", version: 1 },
    "reopened database lost schema metadata",
  );
  await reopenedStore.schemaMetadata.setSchemaMeta({
    name: "contract",
    version: 2,
  });
  await reopenedConnection.closeAsync();
  await SQLite.deleteDatabaseAsync(reopenName);

  return { passed: cases.length + 1 };
}
