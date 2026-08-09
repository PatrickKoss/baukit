import {
  DEFAULT_PAGE_SIZE,
  MAX_PAGE_SIZE,
  type Page,
  type PageOptions,
  type RecordStore,
  type StoredRecord,
} from '@baukit/data-contracts';
import type { SQLiteDatabase } from 'expo-sqlite';

interface StoredRow {
  readonly id: string;
  readonly payload: string;
}

const CURSOR_PREFIX = 'sqlite1:';

/** A namespaced Expo SQLite implementation of Baukit's provider-neutral RecordStore. */
export class SqliteRecordStore<T extends StoredRecord> implements RecordStore<T> {
  public constructor(
    private readonly database: SQLiteDatabase,
    private readonly namespace: string,
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Record store namespace must not be empty.');
    }
  }

  /** Creates the shared adapter table if needed. Safe to call repeatedly. */
  public async initialize(): Promise<void> {
    await this.database.execAsync(
      'CREATE TABLE IF NOT EXISTS baukit_records (namespace TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (namespace, id));',
    );
  }

  public async put(record: T): Promise<void> {
    if (record.id.length === 0) {
      throw new TypeError('Record id must not be empty.');
    }
    await this.database.runAsync(
      'INSERT INTO baukit_records (namespace, id, payload) VALUES (?, ?, ?) ON CONFLICT(namespace, id) DO UPDATE SET payload = excluded.payload',
      this.namespace,
      record.id,
      serializeRecord(record),
    );
  }

  public async get(id: string): Promise<T | undefined> {
    const row = await this.database.getFirstAsync<StoredRow>(
      'SELECT id, payload FROM baukit_records WHERE namespace = ? AND id = ?',
      this.namespace,
      id,
    );
    return row === null ? undefined : (parseRecord(row.payload) as T);
  }

  public async delete(id: string): Promise<void> {
    await this.database.runAsync(
      'DELETE FROM baukit_records WHERE namespace = ? AND id = ?',
      this.namespace,
      id,
    );
  }

  public async list(options: PageOptions = {}): Promise<Page<T>> {
    const limit = pageLimit(options.limit);
    const afterId = decodeCursor(options.cursor);
    const rows = await this.database.getAllAsync<StoredRow>(
      'SELECT id, payload FROM baukit_records WHERE namespace = ? AND id > ? ORDER BY id ASC LIMIT ?',
      this.namespace,
      afterId,
      limit + 1,
    );
    const hasNext = rows.length > limit;
    const pageRows = rows.slice(0, limit);
    const last = pageRows.at(-1);
    return {
      items: pageRows.map((row) => parseRecord(row.payload) as T),
      nextCursor: hasNext && last !== undefined ? encodeCursor(last.id) : null,
    };
  }
}

function pageLimit(requested: number | undefined): number {
  const limit = requested ?? DEFAULT_PAGE_SIZE;
  if (!Number.isInteger(limit) || limit < 1 || limit > MAX_PAGE_SIZE) {
    throw new RangeError(`Page limit must be an integer from 1 to ${String(MAX_PAGE_SIZE)}.`);
  }
  return limit;
}

function encodeCursor(id: string): string {
  return `${CURSOR_PREFIX}${encodeURIComponent(id)}`;
}

function decodeCursor(cursor: string | null | undefined): string {
  if (cursor === undefined || cursor === null) {
    return '';
  }
  if (!cursor.startsWith(CURSOR_PREFIX)) {
    throw new TypeError('Invalid record cursor.');
  }
  try {
    return decodeURIComponent(cursor.slice(CURSOR_PREFIX.length));
  } catch {
    throw new TypeError('Invalid record cursor.');
  }
}

function serializeRecord(record: StoredRecord): string {
  try {
    return JSON.stringify(record);
  } catch {
    throw new TypeError('Record must be JSON serializable.');
  }
}

function parseRecord(payload: string): StoredRecord {
  try {
    const value: unknown = JSON.parse(payload);
    if (
      typeof value !== 'object' ||
      value === null ||
      typeof (value as Record<string, unknown>)['id'] !== 'string'
    ) {
      throw new TypeError('invalid record');
    }
    return value as StoredRecord;
  } catch {
    // Persisted records can contain private product data; never reflect the payload.
    throw new TypeError('The local database contains an invalid record.');
  }
}
