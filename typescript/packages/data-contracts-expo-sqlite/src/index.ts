import {
  DEFAULT_PAGE_SIZE,
  MAX_PAGE_SIZE,
  type JsonValue,
  type KeyValueStore,
  type Page,
  type PageOptions,
  type RecordStore,
  type ReentrantStorageTransaction,
  type SchemaMeta,
  type SchemaMetadataStore,
  StorageError,
  type StoredRecord,
  type TransactionalStorageStore,
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

type SQLiteConnection = Pick<
  SQLiteDatabase,
  'execAsync' | 'getAllAsync' | 'getFirstAsync' | 'runAsync'
>;

interface Lifecycle {
  status: 'closed' | 'closing' | 'open';
}

const CURSOR_PREFIX = 'sqlite1:';

function assertOpen(lifecycle: Lifecycle): void {
  if (lifecycle.status !== 'open') {
    throw new StorageError('storage_closed', 'The storage adapter is closed.');
  }
}

async function write<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
  try {
    return await operation();
  } catch (cause) {
    throw normalizeStorageError(cause);
  }
}

/** A namespaced Expo SQLite implementation of Baukit's provider-neutral RecordStore. */
export class SqliteRecordStore<T extends StoredRecord> implements RecordStore<T> {
  public constructor(
    private readonly database: SQLiteConnection,
    private readonly namespace: string,
    private readonly assertAvailable: () => void = () => undefined,
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Record store namespace must not be empty.');
    }
  }

  /** Creates the shared adapter table if needed. Safe to call repeatedly. */
  public async initialize(): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.execAsync(
        'CREATE TABLE IF NOT EXISTS baukit_records (namespace TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (namespace, id));',
      ),
    );
  }

  public async put(record: T): Promise<void> {
    this.assertAvailable();
    if (record.id.length === 0) {
      throw new TypeError('Record id must not be empty.');
    }
    await write(() =>
      this.database.runAsync(
        'INSERT INTO baukit_records (namespace, id, payload) VALUES (?, ?, ?) ON CONFLICT(namespace, id) DO UPDATE SET payload = excluded.payload',
        this.namespace,
        record.id,
        serialize(record, 'Record'),
      ),
    );
  }

  public async get(id: string): Promise<T | undefined> {
    this.assertAvailable();
    const row = await this.database.getFirstAsync<StoredRow>(
      'SELECT id, payload FROM baukit_records WHERE namespace = ? AND id = ?',
      this.namespace,
      id,
    );
    return row === null ? undefined : (parseRecord(row.payload) as T);
  }

  public async delete(id: string): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.runAsync(
        'DELETE FROM baukit_records WHERE namespace = ? AND id = ?',
        this.namespace,
        id,
      ),
    );
  }

  public async list(options: PageOptions = {}): Promise<Page<T>> {
    this.assertAvailable();
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

/** Namespaced key/value storage used by the composite SQLite adapter. */
export class SqliteKeyValueStore implements KeyValueStore {
  public constructor(
    private readonly database: SQLiteConnection,
    private readonly namespace: string,
    private readonly assertAvailable: () => void = () => undefined,
  ) {}

  public async initialize(): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.execAsync(
        'CREATE TABLE IF NOT EXISTS baukit_key_values (namespace TEXT NOT NULL, key TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (namespace, key));',
      ),
    );
  }

  public async get(key: string): Promise<JsonValue | undefined> {
    this.assertAvailable();
    const row = await this.database.getFirstAsync<KeyValueRow>(
      'SELECT payload FROM baukit_key_values WHERE namespace = ? AND key = ?',
      this.namespace,
      key,
    );
    return row === null ? undefined : parseJson(row.payload, 'key/value');
  }

  public async set(key: string, value: JsonValue): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.runAsync(
        'INSERT INTO baukit_key_values (namespace, key, payload) VALUES (?, ?, ?) ON CONFLICT(namespace, key) DO UPDATE SET payload = excluded.payload',
        this.namespace,
        key,
        serialize(value, 'Value'),
      ),
    );
  }

  public async delete(key: string): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.runAsync(
        'DELETE FROM baukit_key_values WHERE namespace = ? AND key = ?',
        this.namespace,
        key,
      ),
    );
  }

  public async clear(): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.runAsync('DELETE FROM baukit_key_values WHERE namespace = ?', this.namespace),
    );
  }
}

/** Namespaced portable schema metadata used by the composite SQLite adapter. */
export class SqliteSchemaMetadataStore implements SchemaMetadataStore {
  public constructor(
    private readonly database: SQLiteConnection,
    private readonly namespace: string,
    private readonly assertAvailable: () => void = () => undefined,
  ) {}

  public async initialize(): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.database.execAsync(
        'CREATE TABLE IF NOT EXISTS baukit_schema_metadata (namespace TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, version INTEGER NOT NULL);',
      ),
    );
  }

  public async getSchemaMeta(): Promise<SchemaMeta | undefined> {
    this.assertAvailable();
    const row = await this.database.getFirstAsync<SchemaRow>(
      'SELECT name, version FROM baukit_schema_metadata WHERE namespace = ?',
      this.namespace,
    );
    return row === null ? undefined : { name: row.name, version: row.version };
  }

  public async setSchemaMeta(metadata: SchemaMeta): Promise<void> {
    this.assertAvailable();
    validateSchemaMeta(metadata);
    await write(() =>
      this.database.runAsync(
        'INSERT INTO baukit_schema_metadata (namespace, name, version) VALUES (?, ?, ?) ON CONFLICT(namespace) DO UPDATE SET name = excluded.name, version = excluded.version',
        this.namespace,
        metadata.name,
        metadata.version,
      ),
    );
  }
}

class ExpoSqliteTransaction<T extends StoredRecord> implements ReentrantStorageTransaction<T> {
  public readonly keyValues: SqliteKeyValueStore;
  public readonly records: SqliteRecordStore<T>;
  public readonly schemaMetadata: SqliteSchemaMetadataStore;
  private active = true;

  public constructor(database: SQLiteConnection, namespace: string) {
    const assertActive = (): void => {
      if (!this.active) {
        throw new StorageError('storage_closed', 'The transaction context is no longer active.');
      }
    };
    this.keyValues = new SqliteKeyValueStore(database, namespace, assertActive);
    this.records = new SqliteRecordStore<T>(database, namespace, assertActive);
    this.schemaMetadata = new SqliteSchemaMetadataStore(database, namespace, assertActive);
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    if (!this.active) {
      throw new StorageError('storage_closed', 'The transaction context is no longer active.');
    }
    try {
      return await operation(this);
    } catch (cause) {
      throw normalizeStorageError(cause);
    }
  }

  public finish(): void {
    this.active = false;
  }
}

export interface ExpoSqliteStoreOptions {
  /** Close the supplied database when this adapter closes. Defaults to false. */
  readonly closeDatabase?: boolean;
}

/** Complete namespaced adapter using Expo SQLite exclusive write transactions. */
export class ExpoSqliteStore<T extends StoredRecord> implements TransactionalStorageStore<T> {
  public readonly keyValues: SqliteKeyValueStore;
  public readonly records: SqliteRecordStore<T>;
  public readonly schemaMetadata: SqliteSchemaMetadataStore;
  private readonly lifecycle: Lifecycle = { status: 'open' };
  private transactionTail: Promise<void> = Promise.resolve();

  public constructor(
    private readonly database: SQLiteDatabase,
    namespace: string,
    private readonly options: ExpoSqliteStoreOptions = {},
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Storage namespace must not be empty.');
    }
    const assertAvailable = (): void => {
      assertOpen(this.lifecycle);
    };
    this.keyValues = new SqliteKeyValueStore(database, namespace, assertAvailable);
    this.records = new SqliteRecordStore<T>(database, namespace, assertAvailable);
    this.schemaMetadata = new SqliteSchemaMetadataStore(database, namespace, assertAvailable);
    this.namespace = namespace;
  }

  private readonly namespace: string;

  public async initialize(): Promise<void> {
    assertOpen(this.lifecycle);
    await this.keyValues.initialize();
    await this.records.initialize();
    await this.schemaMetadata.initialize();
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    assertOpen(this.lifecycle);
    const previous = this.transactionTail;
    let release = (): void => undefined;
    this.transactionTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;

    let transaction: ExpoSqliteTransaction<T> | undefined;
    try {
      assertOpen(this.lifecycle);
      let outcome: { value: TResult } | undefined;
      await this.database.withExclusiveTransactionAsync(async (connection) => {
        transaction = new ExpoSqliteTransaction<T>(connection, this.namespace);
        outcome = { value: await operation(transaction) };
      });
      if (outcome === undefined) {
        throw new Error('SQLite transaction completed without a callback result.');
      }
      return outcome.value;
    } catch (cause) {
      throw normalizeStorageError(cause);
    } finally {
      transaction?.finish();
      release();
    }
  }

  public async close(): Promise<void> {
    if (this.lifecycle.status === 'closed') {
      return;
    }
    this.lifecycle.status = 'closing';
    await this.transactionTail;
    try {
      if (this.options.closeDatabase === true) {
        await this.database.closeAsync();
      }
    } finally {
      this.lifecycle.status = 'closed';
    }
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
    const payload = JSON.stringify(value);
    return payload;
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
