import { describe, expect, it } from '@jest/globals';
import type { SQLiteDatabase } from 'expo-sqlite';

import { createAppPreferenceRecordStore, createItemRecordStore } from './record-store';

function recordingDatabase() {
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
  return { database, statements, writes };
}

describe('record store seams', () => {
  it('initializes and delegates items to the Baukit Expo SQLite adapter', async () => {
    const { database, statements, writes } = recordingDatabase();

    const store = await createItemRecordStore(database);
    await store.put({ id: 'item-1', name: 'offline item' });

    expect(statements).toHaveLength(1);
    expect(statements[0]).toContain('baukit_records');
    expect(writes).toHaveLength(1);
    expect(writes[0]).toContain('items');
  });

  it('keeps app preferences in their own collection', async () => {
    const { database, writes } = recordingDatabase();

    const store = await createAppPreferenceRecordStore(database);
    await store.put({
      id: 'appearance',
      language: 'system',
      theme: 'dark',
      analytics_consent: 'unknown',
    });

    expect(writes).toHaveLength(1);
    expect(writes[0]).toContain('app-preferences');
  });
});
