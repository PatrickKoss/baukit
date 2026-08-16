import type { JsonValue } from '@baukit/data-contracts';
import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
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

class FakeSQLiteConnection {
  public constructor(protected readonly state: FakeState) {}

  public execAsync(source: string): Promise<void> {
    if (!source.startsWith('CREATE TABLE')) {
      throw new Error('Unexpected fake SQLite statement.');
    }
    return Promise.resolve();
  }

  public runAsync(source: string, ...params: unknown[]): Promise<unknown> {
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
    return Promise.resolve({});
  }

  public getFirstAsync<T>(source: string, ...params: unknown[]): Promise<T | null> {
    const namespace = params[0] as string;
    if (source.includes('FROM baukit_records')) {
      const row = this.state.records.get(`${namespace}\0${params[1] as string}`);
      return Promise.resolve(
        (row === undefined ? null : { id: row.id, payload: row.payload }) as T | null,
      );
    }
    if (source.includes('FROM baukit_key_values')) {
      const payload = this.state.keyValues.get(`${namespace}\0${params[1] as string}`);
      return Promise.resolve((payload === undefined ? null : { payload }) as T | null);
    }
    if (source.includes('FROM baukit_schema_metadata')) {
      return Promise.resolve((this.state.schemaMetadata.get(namespace) ?? null) as T | null);
    }
    throw new Error(`Unexpected fake SQLite statement: ${source}`);
  }

  public getAllAsync<T>(_source: string, ...params: unknown[]): Promise<T[]> {
    const namespace = params[0] as string;
    const afterId = params[1] as string;
    const limit = params[2] as number;
    return Promise.resolve(
      [...this.state.records.values()]
        .filter((row) => row.namespace === namespace && row.id > afterId)
        .sort((left, right) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0))
        .slice(0, limit)
        .map((row) => ({ id: row.id, payload: row.payload }) as T),
    );
  }
}

class FakeSQLiteDatabase extends FakeSQLiteConnection {
  public closed = false;

  public constructor() {
    super(emptyState());
  }

  public get rows(): Map<string, FakeRow> {
    return this.state.records;
  }

  public async withExclusiveTransactionAsync(
    task: (transaction: FakeSQLiteConnection) => Promise<void>,
  ): Promise<void> {
    const pending = copyState(this.state);
    await task(new FakeSQLiteConnection(pending));
    this.state.records.clear();
    this.state.keyValues.clear();
    this.state.schemaMetadata.clear();
    for (const [key, value] of pending.records) this.state.records.set(key, value);
    for (const [key, value] of pending.keyValues) this.state.keyValues.set(key, value);
    for (const [key, value] of pending.schemaMetadata) this.state.schemaMetadata.set(key, value);
  }

  public closeAsync(): Promise<void> {
    this.closed = true;
    return Promise.resolve();
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

async function makeCompositeStore(): Promise<ExpoSqliteStore<ContractTestRecord>> {
  const store = new ExpoSqliteStore<ContractTestRecord>(
    sqlite(new FakeSQLiteDatabase()),
    'contract',
  );
  await store.initialize();
  compositeStores.push(store);
  return store;
}

afterEach(async () => {
  await Promise.all(compositeStores.splice(0).map((store) => store.close()));
});

describeRecordStoreContract(makeRecordStore);
describeKeyValueContract(async () => (await makeCompositeStore()).keyValues);
describeSchemaMetadataContract(async () => (await makeCompositeStore()).schemaMetadata);
describeTransactionalStorageContract(makeCompositeStore);

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
