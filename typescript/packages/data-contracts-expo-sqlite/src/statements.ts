import {
  DEFAULT_PAGE_SIZE,
  MAX_PAGE_SIZE,
  type JsonValue,
  type KeyValueStore,
  type Page,
  type PageOptions,
  type RecordStore,
  type SchemaMeta,
  type SchemaMetadataStore,
  type StoredRecord,
  normalizeStorageError,
} from '@baukit/data-contracts';
import type { SQLiteDatabase } from 'expo-sqlite';

interface StoredRow {
  readonly id: string;
  readonly payload: string;
}

interface KeyValueRow {
  readonly payload: string;
}

interface SchemaRow {
  readonly name: string;
  readonly version: number;
}

export type SQLiteConnection = Pick<
  SQLiteDatabase,
  'execAsync' | 'getAllAsync' | 'getFirstAsync' | 'runAsync'
>;

/** Decides when a statement may be accepted and on which connection it runs. */
export interface StatementScope {
  assertAvailable(): void;
  run<TResult>(statement: (connection: SQLiteConnection) => Promise<TResult>): Promise<TResult>;
}

const CURSOR_PREFIX = 'sqlite1:';

const CREATE_RECORDS_TABLE =
  'CREATE TABLE IF NOT EXISTS baukit_records (namespace TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (namespace, id));';
const CREATE_KEY_VALUES_TABLE =
  'CREATE TABLE IF NOT EXISTS baukit_key_values (namespace TEXT NOT NULL, key TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (namespace, key));';
const CREATE_SCHEMA_METADATA_TABLE =
  'CREATE TABLE IF NOT EXISTS baukit_schema_metadata (namespace TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, version INTEGER NOT NULL);';

export const CREATE_ADAPTER_TABLES = [
  CREATE_KEY_VALUES_TABLE,
  CREATE_RECORDS_TABLE,
  CREATE_SCHEMA_METADATA_TABLE,
].join(' ');

export async function write<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
  try {
    return await operation();
  } catch (cause) {
    throw normalizeStorageError(cause);
  }
}

export class RecordStatements<T extends StoredRecord> implements RecordStore<T> {
  public constructor(
    private readonly scope: StatementScope,
    private readonly namespace: string,
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Record store namespace must not be empty.');
    }
  }

  /** Creates the shared adapter table if needed. Safe to call repeatedly. */
  public async initialize(): Promise<void> {
    this.scope.assertAvailable();
    await write(() => this.scope.run((connection) => connection.execAsync(CREATE_RECORDS_TABLE)));
  }

  public async put(record: T): Promise<void> {
    this.scope.assertAvailable();
    if (record.id.length === 0) {
      throw new TypeError('Record id must not be empty.');
    }
    const payload = serialize(record, 'Record');
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'INSERT INTO baukit_records (namespace, id, payload) VALUES (?, ?, ?) ON CONFLICT(namespace, id) DO UPDATE SET payload = excluded.payload',
          this.namespace,
          record.id,
          payload,
        ),
      ),
    );
  }

  public async get(id: string): Promise<T | undefined> {
    this.scope.assertAvailable();
    const row = await this.scope.run((connection) =>
      connection.getFirstAsync<StoredRow>(
        'SELECT id, payload FROM baukit_records WHERE namespace = ? AND id = ?',
        this.namespace,
        id,
      ),
    );
    return row === null ? undefined : (parseRecord(row.payload) as T);
  }

  public async delete(id: string): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'DELETE FROM baukit_records WHERE namespace = ? AND id = ?',
          this.namespace,
          id,
        ),
      ),
    );
  }

  public async list(options: PageOptions = {}): Promise<Page<T>> {
    this.scope.assertAvailable();
    const limit = pageLimit(options.limit);
    const afterId = decodeCursor(options.cursor);
    const rows = await this.scope.run((connection) =>
      connection.getAllAsync<StoredRow>(
        'SELECT id, payload FROM baukit_records WHERE namespace = ? AND id > ? ORDER BY id ASC LIMIT ?',
        this.namespace,
        afterId,
        limit + 1,
      ),
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

export class KeyValueStatements implements KeyValueStore {
  public constructor(
    private readonly scope: StatementScope,
    private readonly namespace: string,
  ) {}

  public async initialize(): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) => connection.execAsync(CREATE_KEY_VALUES_TABLE)),
    );
  }

  public async get(key: string): Promise<JsonValue | undefined> {
    this.scope.assertAvailable();
    const row = await this.scope.run((connection) =>
      connection.getFirstAsync<KeyValueRow>(
        'SELECT payload FROM baukit_key_values WHERE namespace = ? AND key = ?',
        this.namespace,
        key,
      ),
    );
    return row === null ? undefined : parseJson(row.payload, 'key/value');
  }

  public async set(key: string, value: JsonValue): Promise<void> {
    this.scope.assertAvailable();
    const payload = serialize(value, 'Value');
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'INSERT INTO baukit_key_values (namespace, key, payload) VALUES (?, ?, ?) ON CONFLICT(namespace, key) DO UPDATE SET payload = excluded.payload',
          this.namespace,
          key,
          payload,
        ),
      ),
    );
  }

  public async delete(key: string): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'DELETE FROM baukit_key_values WHERE namespace = ? AND key = ?',
          this.namespace,
          key,
        ),
      ),
    );
  }

  public async clear(): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync('DELETE FROM baukit_key_values WHERE namespace = ?', this.namespace),
      ),
    );
  }

  public async clearPrefix(prefix: string): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'DELETE FROM baukit_key_values WHERE namespace = ? AND substr(CAST(key AS BLOB), 1, length(CAST(? AS BLOB))) = CAST(? AS BLOB)',
          this.namespace,
          prefix,
          prefix,
        ),
      ),
    );
  }
}

export class SchemaMetadataStatements implements SchemaMetadataStore {
  public constructor(
    private readonly scope: StatementScope,
    private readonly namespace: string,
  ) {}

  public async initialize(): Promise<void> {
    this.scope.assertAvailable();
    await write(() =>
      this.scope.run((connection) => connection.execAsync(CREATE_SCHEMA_METADATA_TABLE)),
    );
  }

  public async getSchemaMeta(): Promise<SchemaMeta | undefined> {
    this.scope.assertAvailable();
    const row = await this.scope.run((connection) =>
      connection.getFirstAsync<SchemaRow>(
        'SELECT name, version FROM baukit_schema_metadata WHERE namespace = ?',
        this.namespace,
      ),
    );
    return row === null ? undefined : { name: row.name, version: row.version };
  }

  public async setSchemaMeta(metadata: SchemaMeta): Promise<void> {
    this.scope.assertAvailable();
    validateSchemaMeta(metadata);
    await write(() =>
      this.scope.run((connection) =>
        connection.runAsync(
          'INSERT INTO baukit_schema_metadata (namespace, name, version) VALUES (?, ?, ?) ON CONFLICT(namespace) DO UPDATE SET name = excluded.name, version = excluded.version',
          this.namespace,
          metadata.name,
          metadata.version,
        ),
      ),
    );
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

function serialize(value: unknown, kind: string): string {
  try {
    return JSON.stringify(value);
  } catch {
    throw new TypeError(`${kind} must be JSON serializable.`);
  }
}

function parseJson(payload: string, kind: string): JsonValue {
  try {
    return JSON.parse(payload) as JsonValue;
  } catch {
    throw new TypeError(`The local database contains an invalid ${kind} value.`);
  }
}

function parseRecord(payload: string): StoredRecord {
  try {
    const value: unknown = JSON.parse(payload);
    if (
      typeof value !== 'object' ||
      value === null ||
      Array.isArray(value) ||
      typeof Reflect.get(value, 'id') !== 'string'
    ) {
      throw new TypeError('invalid record');
    }
    return value as StoredRecord;
  } catch {
    throw new TypeError('The local database contains an invalid record.');
  }
}

function validateSchemaMeta(metadata: SchemaMeta): void {
  if (metadata.name.length === 0) {
    throw new TypeError('Schema name must not be empty.');
  }
  if (!Number.isInteger(metadata.version) || metadata.version < 0) {
    throw new TypeError('Schema version must be a non-negative integer.');
  }
}
