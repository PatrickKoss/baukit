import type { JsonValue } from '@baukit/data-contracts';
import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeScopedPersistenceContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';
import type { SQLiteDatabase } from 'expo-sqlite';
import { afterEach, describe, expect, it } from 'vitest';

import { ExpoSqliteStore, SqliteRecordStore } from './index.js';

interface FakeRow {
  readonly namespace: string;
  readonly id: string;
  payload: string;
}

interface FakeState {
  readonly records: Map<string, FakeRow>;
  readonly keyValues: Map<string, string>;
  readonly schemaMetadata: Map<string, { readonly name: string; readonly version: number }>;
}

type ConnectionRole = 'root' | 'transaction';

interface Signal {
  readonly promise: Promise<void>;
  readonly resolve: () => void;
}

interface Pause {
  readonly started: Promise<void>;
  release(): void;
}

function signal(): Signal {
  let resolve = (): void => undefined;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function flush(): Promise<void> {
  return new Promise((resolve) => {
    setImmediate(resolve);
  });
}

function emptyState(): FakeState {
  return {
    records: new Map(),
    keyValues: new Map(),
    schemaMetadata: new Map(),
  };
}

function copyState(state: FakeState): FakeState {
  return {
    records: new Map([...state.records].map(([key, value]) => [key, { ...value }])),
    keyValues: new Map(state.keyValues),
    schemaMetadata: new Map([...state.schemaMetadata].map(([key, value]) => [key, { ...value }])),
  };
}

function replaceState(target: FakeState, source: FakeState): void {
  target.records.clear();
  target.keyValues.clear();
  target.schemaMetadata.clear();
  for (const [key, value] of source.records) target.records.set(key, value);
  for (const [key, value] of source.keyValues) target.keyValues.set(key, value);
  for (const [key, value] of source.schemaMetadata) target.schemaMetadata.set(key, value);
}

function databaseLocked(): Error {
  return new Error('database is locked');
}

/**
 * One SQLite file shared by every fake connection opened on it. Like real SQLite without a busy
 * timeout, a statement on a root connection fails while another connection holds an exclusive
 * transaction, and a transaction cannot begin while a root statement is in flight.
 */
class FakeSqliteFile {
  public readonly state = emptyState();
  public statementAttempts = 0;
  private rootStatements = 0;
  private transactionOpen = false;
  private readonly pauses: { readonly started: () => void; readonly gate: Promise<void> }[] = [];
  private readonly failures: unknown[] = [];

  public pauseNextStatement(): Pause {
    const started = signal();
    const gate = signal();
    this.pauses.push({ started: started.resolve, gate: gate.promise });
    return { started: started.promise, release: gate.resolve };
  }

  public failNextStatement(cause: unknown): void {
    this.failures.push(cause);
  }

  public async runStatement<TResult>(
    role: ConnectionRole,
    execute: () => TResult,
  ): Promise<TResult> {
    this.statementAttempts += 1;
    this.acquire(role);
    try {
      await this.pauseOrYield();
      if (this.failures.length > 0) {
        throw this.failures.shift();
      }
      return execute();
    } finally {
      this.release(role);
    }
  }

  public beginTransaction(): void {
    if (this.transactionOpen || this.rootStatements > 0) {
      throw databaseLocked();
    }
    this.transactionOpen = true;
  }

  public endTransaction(): void {
    this.transactionOpen = false;
  }

  private acquire(role: ConnectionRole): void {
    if (role === 'transaction') return;
    if (this.transactionOpen) {
      throw databaseLocked();
    }
    this.rootStatements += 1;
  }

  private release(role: ConnectionRole): void {
    if (role === 'root') this.rootStatements -= 1;
  }

  private async pauseOrYield(): Promise<void> {
    const pause = this.pauses.shift();
    if (pause === undefined) {
      await Promise.resolve();
      return;
    }
    pause.started();
    await pause.gate;
  }
}

class FakeSQLiteConnection {
  public constructor(
    public readonly file: FakeSqliteFile,
    private readonly role: ConnectionRole,
    private readonly state: FakeState,
  ) {}

  public execAsync(source: string): Promise<void> {
    return this.file.runStatement(this.role, () => {
      if (!source.startsWith('CREATE TABLE')) {
        throw new Error('Unexpected fake SQLite statement.');
      }
    });
  }

  public runAsync(source: string, ...params: unknown[]): Promise<unknown> {
    return this.file.runStatement(this.role, () => {
      this.apply(source, params);
      return {};
    });
  }

  public getFirstAsync<T>(source: string, ...params: unknown[]): Promise<T | null> {
    return this.file.runStatement(this.role, () => this.first(source, params) as T | null);
  }

  public getAllAsync<T>(_source: string, ...params: unknown[]): Promise<T[]> {
    const namespace = params[0] as string;
    const afterId = params[1] as string;
    const limit = params[2] as number;
    return this.file.runStatement(this.role, () =>
      [...this.state.records.values()]
        .filter((row) => row.namespace === namespace && row.id > afterId)
        .sort((left, right) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0))
        .slice(0, limit)
        .map((row) => ({ id: row.id, payload: row.payload }) as T),
    );
  }

  private apply(source: string, params: unknown[]): void {
    const namespace = params[0] as string;
    if (source.startsWith('INSERT INTO baukit_records')) {
      const id = params[1] as string;
      this.state.records.set(`${namespace}\0${id}`, {
        namespace,
        id,
        payload: params[2] as string,
      });
    } else if (source.startsWith('DELETE FROM baukit_records')) {
      this.state.records.delete(`${namespace}\0${params[1] as string}`);
    } else if (source.startsWith('INSERT INTO baukit_key_values')) {
      this.state.keyValues.set(`${namespace}\0${params[1] as string}`, params[2] as string);
    } else if (source === 'DELETE FROM baukit_key_values WHERE namespace = ?') {
      for (const key of this.state.keyValues.keys()) {
        if (key.startsWith(`${namespace}\0`)) {
          this.state.keyValues.delete(key);
        }
      }
    } else if (source.startsWith('DELETE FROM baukit_key_values')) {
      this.state.keyValues.delete(`${namespace}\0${params[1] as string}`);
    } else if (source.startsWith('INSERT INTO baukit_schema_metadata')) {
      this.state.schemaMetadata.set(namespace, {
        name: params[1] as string,
        version: params[2] as number,
      });
    } else {
      throw new Error(`Unexpected fake SQLite statement: ${source}`);
    }
  }

  private first(source: string, params: unknown[]): unknown {
    const namespace = params[0] as string;
    if (source.includes('FROM baukit_records')) {
      const row = this.state.records.get(`${namespace}\0${params[1] as string}`);
      return row === undefined ? null : { id: row.id, payload: row.payload };
    }
    if (source.includes('FROM baukit_key_values')) {
      const payload = this.state.keyValues.get(`${namespace}\0${params[1] as string}`);
      return payload === undefined ? null : { payload };
    }
    if (source.includes('FROM baukit_schema_metadata')) {
      return this.state.schemaMetadata.get(namespace) ?? null;
    }
    throw new Error(`Unexpected fake SQLite statement: ${source}`);
  }
}

let fakePathSequence = 0;

function uniquePath(): string {
  fakePathSequence += 1;
  return `/fake/database-${String(fakePathSequence)}.db`;
}

class FakeSQLiteDatabase extends FakeSQLiteConnection {
  public closeCount = 0;
  private closeFailure: Error | undefined;

  public constructor(
    file: FakeSqliteFile = new FakeSqliteFile(),
    public readonly databasePath: string = uniquePath(),
  ) {
    super(file, 'root', file.state);
  }

  public get rows(): Map<string, FakeRow> {
    return this.file.state.records;
  }

  public async withExclusiveTransactionAsync(
    task: (transaction: FakeSQLiteConnection) => Promise<void>,
  ): Promise<void> {
    this.file.beginTransaction();
    try {
      const pending = copyState(this.file.state);
      await task(new FakeSQLiteConnection(this.file, 'transaction', pending));
      replaceState(this.file.state, pending);
    } finally {
      this.file.endTransaction();
    }
  }

  public failClose(cause: Error): void {
    this.closeFailure = cause;
  }

  public closeAsync(): Promise<void> {
    this.closeCount += 1;
    return this.closeFailure === undefined ? Promise.resolve() : Promise.reject(this.closeFailure);
  }
}

function sqlite(database: FakeSQLiteDatabase): SQLiteDatabase {
  return database as unknown as SQLiteDatabase;
}

async function makeRecordStore(): Promise<SqliteRecordStore<ContractTestRecord>> {
  const store = new SqliteRecordStore<ContractTestRecord>(
    sqlite(new FakeSQLiteDatabase()),
    'contract',
  );
  await store.initialize();
  return store;
}

const compositeStores: ExpoSqliteStore<ContractTestRecord>[] = [];

async function openCompositeStore(
  database: FakeSQLiteDatabase = new FakeSQLiteDatabase(),
  namespace = 'contract',
  closeDatabase = false,
): Promise<ExpoSqliteStore<ContractTestRecord>> {
  const store = new ExpoSqliteStore<ContractTestRecord>(sqlite(database), namespace, {
    closeDatabase,
  });
  await store.initialize();
  compositeStores.push(store);
  return store;
}

function makeCompositeStore(): Promise<ExpoSqliteStore<ContractTestRecord>> {
  return openCompositeStore();
}

afterEach(async () => {
  await Promise.allSettled(compositeStores.splice(0).map((store) => store.close()));
});

describeRecordStoreContract(makeRecordStore);
describeKeyValueContract(async () => (await makeCompositeStore()).keyValues);
describeSchemaMetadataContract(async () => (await makeCompositeStore()).schemaMetadata);
describeTransactionalStorageContract(makeCompositeStore);
describeScopedPersistenceContract(() => {
  const files = new Map<string, FakeSqliteFile>();
  return {
    open: (storeName) => {
      let file = files.get(storeName);
      if (file === undefined) {
        file = new FakeSqliteFile();
        files.set(storeName, file);
      }
      return openCompositeStore(new FakeSQLiteDatabase(file, `/fake/${storeName}.db`));
    },
  };
});

describe('SQLite adapters', () => {
  it('isolates records in separate namespaces sharing a database', async () => {
    const database = new FakeSQLiteDatabase();
    const first = new SqliteRecordStore<ContractTestRecord>(sqlite(database), 'first');
    const second = new SqliteRecordStore<ContractTestRecord>(sqlite(database), 'second');
    await first.initialize();
    await first.put({ id: 'same', label: 'first', payload: 1 });
    await second.put({ id: 'same', label: 'second', payload: 2 });
    await expect(first.get('same')).resolves.toMatchObject({ label: 'first' });
    await expect(second.get('same')).resolves.toMatchObject({ label: 'second' });
  });

  it('does not include malformed private payload content in errors', async () => {
    const database = new FakeSQLiteDatabase();
    database.rows.set('private\0record', {
      namespace: 'private',
      id: 'record',
      payload: 'private journal content {',
    });
    const store = new SqliteRecordStore<ContractTestRecord>(sqlite(database), 'private');
    let caught: unknown;
    try {
      await store.get('record');
    } catch (cause) {
      caught = cause;
    }
    expect(caught).toBeInstanceOf(TypeError);
    expect((caught as Error).message).toBe('The local database contains an invalid record.');
    expect((caught as Error).message).not.toContain('journal');
  });

  it('round-trips JSON key/value shapes in the fake native harness', async () => {
    const store = await makeCompositeStore();
    const value: JsonValue = { nested: [true, null, 'value'] };
    await store.keyValues.set('json', value);
    await expect(store.keyValues.get('json')).resolves.toEqual(value);
  });
});

const RECORD: ContractTestRecord = { id: 'record', label: 'queued', payload: 1 };

type Store = ExpoSqliteStore<ContractTestRecord>;

const ROOT_OPERATIONS: readonly (readonly [string, (store: Store) => Promise<unknown>])[] = [
  ['record write', (store) => store.records.put(RECORD)],
  ['record read', (store) => store.records.get(RECORD.id)],
  ['record list', (store) => store.records.list()],
  ['key/value write', (store) => store.keyValues.set('key', 'value')],
  ['key/value read', (store) => store.keyValues.get('key')],
  [
    'schema metadata write',
    (store) => store.schemaMetadata.setSchemaMeta({ name: 's', version: 1 }),
  ],
  ['schema metadata read', (store) => store.schemaMetadata.getSchemaMeta()],
];

interface HeldTransaction {
  readonly done: Promise<void>;
  release(): void;
}

async function holdTransaction(store: Store): Promise<HeldTransaction> {
  const entered = signal();
  const gate = signal();
  const done = store.withTransaction(async (transaction) => {
    await transaction.keyValues.set('held', true);
    entered.resolve();
    await gate.promise;
  });
  await entered.promise;
  return { done, release: gate.resolve };
}

function track(operation: Promise<unknown>): { readonly settled: () => boolean } {
  let settled = false;
  void operation.then(
    () => {
      settled = true;
    },
    () => {
      settled = true;
    },
  );
  return { settled: () => settled };
}

describe('ExpoSqliteStore operation serialization', () => {
  it.each(ROOT_OPERATIONS)(
    'starts a transaction only after an active %s finishes',
    async (_name, operation) => {
      const database = new FakeSQLiteDatabase();
      const store = await openCompositeStore(database);
      const pause = database.file.pauseNextStatement();
      const root = operation(store);
      await pause.started;

      let transactionStarted = false;
      const transaction = store.withTransaction(async (context) => {
        transactionStarted = true;
        await context.keyValues.set('after', 'root');
      });
      await flush();
      const startedEarly = transactionStarted;
      pause.release();

      await root;
      await expect(transaction).resolves.toBeUndefined();
      expect(startedEarly).toBe(false);
      await expect(store.keyValues.get('after')).resolves.toBe('root');
    },
  );

  it.each(ROOT_OPERATIONS)(
    'runs a %s only after an active transaction commits',
    async (_name, operation) => {
      const store = await openCompositeStore();
      const held = await holdTransaction(store);

      const root = operation(store);
      const progress = track(root);
      await flush();
      const settledEarly = progress.settled();
      held.release();

      await expect(held.done).resolves.toBeUndefined();
      await root;
      expect(settledEarly).toBe(false);
      await expect(store.keyValues.get('held')).resolves.toBe(true);
    },
  );

  it('orders initialize with a transaction that arrives first', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database);
    const held = await holdTransaction(store);
    const initialize = store.initialize();
    const progress = track(initialize);
    await flush();
    expect(progress.settled()).toBe(false);
    held.release();
    await expect(initialize).resolves.toBeUndefined();
  });

  it('releases the queue after a rejected root operation', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database);
    const failure = new Error('disk I/O error');
    database.file.failNextStatement(failure);

    const failed = store.records.get(RECORD.id);
    const queued = store.records.put(RECORD);

    await expect(failed).rejects.toBe(failure);
    await expect(queued).resolves.toBeUndefined();
    await expect(store.records.get(RECORD.id)).resolves.toEqual(RECORD);
  });

  it('rolls back a rejected transaction and releases the queue', async () => {
    const store = await openCompositeStore();
    const failed = store.withTransaction(async (transaction) => {
      await transaction.records.put(RECORD);
      throw new Error('rollback');
    });
    const queued = store.records.get(RECORD.id);

    await expect(failed).rejects.toThrow('rollback');
    await expect(queued).resolves.toBeUndefined();
    await expect(store.records.put(RECORD)).resolves.toBeUndefined();
  });

  it('surfaces a lock error unchanged without retrying the statement', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database);
    const locked = databaseLocked();
    database.file.failNextStatement(locked);
    const attemptsBefore = database.file.statementAttempts;

    await expect(store.keyValues.set('key', 'value')).rejects.toBe(locked);
    expect(database.file.statementAttempts - attemptsBefore).toBe(1);
    await expect(store.keyValues.get('key')).resolves.toBeUndefined();
  });

  it('keeps nested transaction work on the transaction connection', async () => {
    const store = await openCompositeStore();
    const queuedBehind = store.keyValues.set('after', 'nested');

    await store.withTransaction((transaction) =>
      transaction.withTransaction(async (nested) => {
        await nested.records.put(RECORD);
        await nested.keyValues.set('nested', true);
        await nested.schemaMetadata.setSchemaMeta({ name: 'nested', version: 2 });
        return nested.withTransaction((deeper) => deeper.records.get(RECORD.id));
      }),
    );

    await expect(queuedBehind).resolves.toBeUndefined();
    await expect(store.records.get(RECORD.id)).resolves.toEqual(RECORD);
    await expect(store.schemaMetadata.getSchemaMeta()).resolves.toEqual({
      name: 'nested',
      version: 2,
    });
  });

  it('drains accepted work before closing and rejects later calls', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database, 'contract', true);
    const pause = database.file.pauseNextStatement();
    const write = store.records.put(RECORD);
    await pause.started;
    const acceptedTransaction = store.withTransaction((transaction) =>
      transaction.keyValues.set('accepted', true),
    );

    const close = store.close();
    const repeatedClose = store.close();
    const progress = track(close);
    await expect(store.records.get(RECORD.id)).rejects.toMatchObject({ code: 'storage_closed' });
    await expect(store.withTransaction(() => undefined)).rejects.toMatchObject({
      code: 'storage_closed',
    });
    await expect(store.initialize()).rejects.toMatchObject({ code: 'storage_closed' });
    await flush();
    const closedEarly = progress.settled();
    pause.release();

    await expect(write).resolves.toBeUndefined();
    await expect(acceptedTransaction).resolves.toBeUndefined();
    await expect(close).resolves.toBeUndefined();
    expect(repeatedClose).toBe(close);
    expect(closedEarly).toBe(false);
    expect(database.closeCount).toBe(1);
    expect(database.file.state.keyValues.get('contract\0accepted')).toBe('true');
    await expect(store.keyValues.get('accepted')).rejects.toMatchObject({
      code: 'storage_closed',
    });
  });

  it('shares one failed close result and closes the database once', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database, 'contract', true);
    const failure = new Error('close failed');
    database.failClose(failure);

    const close = store.close();
    const repeatedClose = store.close();

    expect(repeatedClose).toBe(close);
    await expect(close).rejects.toBe(failure);
    await expect(store.close()).rejects.toBe(failure);
    expect(database.closeCount).toBe(1);
    await expect(store.keyValues.get('key')).rejects.toMatchObject({ code: 'storage_closed' });
  });

  it('leaves a supplied database open unless the adapter owns it', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database);
    await store.close();
    expect(database.closeCount).toBe(0);
  });
});

describe('shared database coordination', () => {
  it('orders stores in different namespaces on one database handle', async () => {
    const database = new FakeSQLiteDatabase();
    const first = await openCompositeStore(database, 'first');
    const second = await openCompositeStore(database, 'second');
    const held = await holdTransaction(first);

    const write = second.records.put(RECORD);
    const progress = track(write);
    await flush();
    const settledEarly = progress.settled();
    held.release();

    await expect(write).resolves.toBeUndefined();
    await expect(held.done).resolves.toBeUndefined();
    expect(settledEarly).toBe(false);
  });

  it('orders stores on separate handles to the same database file', async () => {
    const file = new FakeSqliteFile();
    const first = await openCompositeStore(new FakeSQLiteDatabase(file, '/fake/shared.db'));
    const second = await openCompositeStore(new FakeSQLiteDatabase(file, '/fake/shared.db'));
    const held = await holdTransaction(first);

    const read = second.keyValues.get('held');
    const progress = track(read);
    await flush();
    const settledEarly = progress.settled();
    held.release();

    await expect(read).resolves.toBe(true);
    expect(settledEarly).toBe(false);
  });

  it('orders a standalone record store with a transaction on the same database', async () => {
    const database = new FakeSQLiteDatabase();
    const store = await openCompositeStore(database);
    const records = new SqliteRecordStore<ContractTestRecord>(sqlite(database), 'standalone');
    await records.initialize();
    const held = await holdTransaction(store);

    const write = records.put(RECORD);
    const progress = track(write);
    await flush();
    const settledEarly = progress.settled();
    held.release();

    await expect(write).resolves.toBeUndefined();
    expect(settledEarly).toBe(false);
  });

  it('lets independent databases proceed while another holds a transaction', async () => {
    const busy = await openCompositeStore(new FakeSQLiteDatabase());
    const independent = await openCompositeStore(new FakeSQLiteDatabase());
    const held = await holdTransaction(busy);

    const write = independent.records.put(RECORD);
    await expect(write).resolves.toBeUndefined();
    await expect(independent.withTransaction(() => 'independent')).resolves.toBe('independent');

    held.release();
    await expect(held.done).resolves.toBeUndefined();
  });
});
