import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeScopedPersistenceContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';
import { afterEach, describe, expect, it } from 'vitest';

import { ExpoSqliteStore } from './index.js';
import { NodeSqliteDatabase } from './testing.js';

interface ValueRow {
  readonly value: unknown;
  readonly kind: string;
}

const databases: NodeSqliteDatabase[] = [];
const directories: string[] = [];
const stores: ExpoSqliteStore<ContractTestRecord>[] = [];

function openDatabase(path?: string): NodeSqliteDatabase {
  const database = new NodeSqliteDatabase(path);
  databases.push(database);
  return database;
}

function temporaryDirectory(): string {
  const directory = mkdtempSync(join(tmpdir(), 'baukit-node-sqlite-test-'));
  directories.push(directory);
  return directory;
}

async function openValuesTable(): Promise<NodeSqliteDatabase> {
  const database = openDatabase();
  await database.execAsync('CREATE TABLE value_table (value ANY) STRICT');
  return database;
}

async function storedValue(database: NodeSqliteDatabase): Promise<ValueRow | null> {
  return database.getFirstAsync<ValueRow>(
    'SELECT value, typeof(value) AS kind FROM value_table ORDER BY rowid DESC LIMIT 1',
  );
}

async function openStore(
  database: NodeSqliteDatabase,
): Promise<ExpoSqliteStore<ContractTestRecord>> {
  const store = new ExpoSqliteStore<ContractTestRecord>(database, 'contract', {
    closeDatabase: true,
  });
  await store.initialize();
  stores.push(store);
  return store;
}

afterEach(async () => {
  await Promise.allSettled(stores.splice(0).map((store) => store.close()));
  await Promise.allSettled(databases.splice(0).map((database) => database.closeAsync()));
  for (const directory of directories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe('NodeSqliteDatabase', () => {
  it('binds variadic, array, and prefixed named parameters', async () => {
    const database = await openValuesTable();
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', 'variadic');
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', ['array']);
    await database.runAsync('INSERT INTO value_table (value) VALUES ($value)', { $value: 'named' });
    const rows = await database.getAllAsync<{ readonly value: string }>(
      'SELECT value FROM value_table ORDER BY rowid',
    );
    expect(rows).toEqual([{ value: 'variadic' }, { value: 'array' }, { value: 'named' }]);
  });

  it('stores booleans and integral numbers as integers, as the native module does', async () => {
    const database = await openValuesTable();
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', true);
    await expect(storedValue(database)).resolves.toEqual({ value: 1, kind: 'integer' });
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', 42);
    await expect(storedValue(database)).resolves.toEqual({ value: 42, kind: 'integer' });
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', 1.5);
    await expect(storedValue(database)).resolves.toEqual({ value: 1.5, kind: 'real' });
  });

  it('binds undefined as NULL and a single blob as one value', async () => {
    const database = await openValuesTable();
    // @ts-expect-error expo-sqlite's types forbid undefined, but untyped callers can still pass it.
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', [undefined]);
    await expect(storedValue(database)).resolves.toEqual({ value: null, kind: 'null' });
    await database.runAsync('INSERT INTO value_table (value) VALUES (?)', new Uint8Array([1, 2]));
    await expect(storedValue(database)).resolves.toEqual({
      value: new Uint8Array([1, 2]),
      kind: 'blob',
    });
  });

  it('requires the prefix on named parameters', async () => {
    const database = await openValuesTable();
    await expect(
      database.runAsync('INSERT INTO value_table (value) VALUES ($value)', { value: 'bare' }),
    ).rejects.toThrow();
  });

  it('returns plain row objects, null for no row, and the run result', async () => {
    const database = await openValuesTable();
    const result = await database.runAsync('INSERT INTO value_table (value) VALUES (?)', 'row');
    expect(result).toEqual({ changes: 1, lastInsertRowId: 1 });
    const row = await database.getFirstAsync('SELECT value FROM value_table');
    expect(Object.getPrototypeOf(row)).toBe(Object.prototype);
    await expect(
      database.getFirstAsync('SELECT value FROM value_table WHERE value = ?', 'absent'),
    ).resolves.toBeNull();
  });

  it('commits an exclusive transaction on a second connection to the same file', async () => {
    const database = await openValuesTable();
    await database.withExclusiveTransactionAsync(async (transaction) => {
      await transaction.runAsync('INSERT INTO value_table (value) VALUES (?)', 'committed');
    });
    await expect(storedValue(database)).resolves.toMatchObject({ value: 'committed' });
  });

  it('rolls back an exclusive transaction when its task throws', async () => {
    const database = await openValuesTable();
    const failure = new Error('task failed');
    await expect(
      database.withExclusiveTransactionAsync(async (transaction) => {
        await transaction.runAsync('INSERT INTO value_table (value) VALUES (?)', 'discarded');
        throw failure;
      }),
    ).rejects.toBe(failure);
    await expect(storedValue(database)).resolves.toBeNull();
  });

  it('fails a root write that overlaps an open exclusive transaction', async () => {
    const database = await openValuesTable();
    await database.withExclusiveTransactionAsync(async (transaction) => {
      await transaction.runAsync('INSERT INTO value_table (value) VALUES (?)', 'inside');
      await expect(
        database.runAsync('INSERT INTO value_table (value) VALUES (?)', 'outside'),
      ).rejects.toThrow('database is locked');
    });
  });

  it('leaves foreign keys off, including on transaction connections', async () => {
    const database = openDatabase();
    await database.execAsync(`CREATE TABLE parent (id TEXT PRIMARY KEY);
CREATE TABLE child (id TEXT PRIMARY KEY, parent_id TEXT REFERENCES parent(id));
PRAGMA foreign_keys = ON;`);
    await database.withExclusiveTransactionAsync(async (transaction) => {
      await transaction.runAsync('INSERT INTO child (id, parent_id) VALUES (?, ?)', 'a', 'none');
    });
    await expect(
      database.runAsync('INSERT INTO child (id, parent_id) VALUES (?, ?)', 'b', 'none'),
    ).rejects.toThrow('FOREIGN KEY constraint failed');
  });

  it('rejects an in-memory path', () => {
    expect(() => new NodeSqliteDatabase(':memory:')).toThrow(TypeError);
  });

  it('deletes its temporary file on close and keeps a caller-owned file', async () => {
    const owned = new NodeSqliteDatabase();
    await owned.closeAsync();
    expect(existsSync(dirname(owned.databasePath))).toBe(false);

    const path = join(temporaryDirectory(), 'kept.sqlite');
    const kept = new NodeSqliteDatabase(path);
    await kept.closeAsync();
    expect(existsSync(path)).toBe(true);
  });
});

describe('ExpoSqliteStore on NodeSqliteDatabase', () => {
  const makeStore = (): Promise<ExpoSqliteStore<ContractTestRecord>> =>
    openStore(new NodeSqliteDatabase());

  describeRecordStoreContract(async () => (await makeStore()).records);
  describeKeyValueContract(async () => (await makeStore()).keyValues);
  describeSchemaMetadataContract(async () => (await makeStore()).schemaMetadata);
  describeTransactionalStorageContract(makeStore);
  describeScopedPersistenceContract(() => {
    const directory = temporaryDirectory();
    return {
      open: (storeName) => openStore(new NodeSqliteDatabase(join(directory, `${storeName}.db`))),
    };
  });
});
