import {
  InMemoryScopedPersistenceRegistryStore,
  MAX_PAGE_SIZE,
  ScopedPersistenceLifecycle,
  type JsonValue,
  type StoredRecord,
  recheckServerSubjectBeforeSyncAdoption,
} from "@baukit/data-contracts";
import { ExpoSqliteStore } from "@baukit/data-contracts-expo-sqlite";
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
    for (const testCase of cases) await testCase.run();
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
