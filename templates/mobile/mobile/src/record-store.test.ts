import { describe, expect, it } from '@jest/globals';
import type { SQLiteDatabase } from 'expo-sqlite';

import { createItemRecordStore } from './record-store';

describe('item record store seam', () => {
  it('initializes and delegates records to the Baukit Expo SQLite adapter', async () => {
    const statements: string[] = [];
    const writes: (readonly unknown[])[] = [];
    const database = {
      execAsync: (statement: string) => {
        statements.push(statement);
        return Promise.resolve();
      },
      runAsync: (...parameters: readonly unknown[]) => {
        writes.push(parameters);
        return Promise.resolve({ changes: 1, lastInsertRowId: 0 });
      },
      getFirstAsync: () => Promise.resolve(null),
      getAllAsync: () => Promise.resolve([]),
    } as unknown as SQLiteDatabase;

    const store = await createItemRecordStore(database);
    await store.put({ id: 'item-1', name: 'offline item' });

    expect(statements).toHaveLength(1);
    expect(statements[0]).toContain('baukit_records');
    expect(writes).toHaveLength(1);
  });
});
