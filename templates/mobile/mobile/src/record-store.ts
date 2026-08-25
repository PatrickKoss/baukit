import { SqliteRecordStore } from '@baukit/data-contracts-expo-sqlite';
import type { RecordStore } from '@baukit/data-contracts';
import type { SQLiteDatabase } from 'expo-sqlite';

import type { Item } from './api';
import type { AppPreferenceRecord } from './app-preferences';

/** Creates the product's replaceable item-storage seam on an Expo SQLite database. */
export async function createItemRecordStore(database: SQLiteDatabase): Promise<RecordStore<Item>> {
  const store = new SqliteRecordStore<Item>(database, 'items');
  await store.initialize();
  return store;
}

export async function createAppPreferenceRecordStore(
  database: SQLiteDatabase,
): Promise<RecordStore<AppPreferenceRecord>> {
  const store = new SqliteRecordStore<AppPreferenceRecord>(database, 'app-preferences');
  await store.initialize();
  return store;
}
