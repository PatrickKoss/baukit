import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import { describeRecordStoreContract } from '@baukit/data-contracts/vitest';
import type { SQLiteDatabase } from 'expo-sqlite';
import { describe, expect, it } from 'vitest';

import { SqliteRecordStore } from './index.js';

interface FakeRow {
  readonly namespace: string;
  readonly id: string;
  payload: string;
}

class FakeSQLiteDatabase {
  public readonly rows = new Map<string, FakeRow>();

  public execAsync(source: string): Promise<void> {
    if (!source.startsWith('CREATE TABLE')) {
      throw new Error('Unexpected fake SQLite statement.');
    }
    return Promise.resolve();
  }

  public runAsync(source: string, ...params: unknown[]): Promise<unknown> {
    const namespace = params[0] as string;
    const id = params[1] as string;
    if (source.startsWith('INSERT')) {
      this.rows.set(`${namespace}\0${id}`, {
        namespace,
        id,
        payload: params[2] as string,
      });
    } else if (source.startsWith('DELETE')) {
      this.rows.delete(`${namespace}\0${id}`);
    } else {
      throw new Error('Unexpected fake SQLite statement.');
    }
    return Promise.resolve({});
  }

  public getFirstAsync<T>(_source: string, ...params: unknown[]): Promise<T | null> {
    const row = this.rows.get(`${params[0] as string}\0${params[1] as string}`);
    return Promise.resolve(
      (row === undefined ? null : { id: row.id, payload: row.payload }) as T | null,
    );
  }

  public getAllAsync<T>(_source: string, ...params: unknown[]): Promise<T[]> {
    const namespace = params[0] as string;
    const afterId = params[1] as string;
    const limit = params[2] as number;
    return Promise.resolve(
      [...this.rows.values()]
        .filter((row) => row.namespace === namespace && row.id > afterId)
        .sort((left, right) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0))
        .slice(0, limit)
        .map((row) => ({ id: row.id, payload: row.payload }) as T),
    );
  }
}

function sqlite(database: FakeSQLiteDatabase): SQLiteDatabase {
  return database as unknown as SQLiteDatabase;
}

async function makeStore(): Promise<SqliteRecordStore<ContractTestRecord>> {
  const store = new SqliteRecordStore<ContractTestRecord>(
    sqlite(new FakeSQLiteDatabase()),
    'contract',
  );
  await store.initialize();
  return store;
}

describeRecordStoreContract(makeStore);

describe('SqliteRecordStore', () => {
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
});
