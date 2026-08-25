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
import { Dexie, type DexieOptions, type Table } from 'dexie';

interface KeyValueRow {
  readonly key: string;
  readonly payload: string;
}

interface RecordRow {
  readonly id: string;
  readonly payload: string;
}

interface SchemaRow extends SchemaMeta {
  readonly key: 'schema';
}

interface Lifecycle {
  status: 'closed' | 'closing' | 'open';
}

const CURSOR_PREFIX = 'dexie1:';

class BaukitDexieDatabase extends Dexie {
  public readonly keyValues: Table<KeyValueRow, string>;
  public readonly records: Table<RecordRow, string>;
  public readonly schemaMetadata: Table<SchemaRow, string>;

  public constructor(name: string, options?: DexieOptions) {
    super(name, options);
    this.version(1).stores({
      keyValues: 'key',
      records: 'id',
      schemaMetadata: 'key',
    });
    this.keyValues = this.table('keyValues');
    this.records = this.table('records');
    this.schemaMetadata = this.table('schemaMetadata');
  }
}

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

/** Dexie implementation of the provider-neutral key/value contract. */
export class DexieKeyValueStore implements KeyValueStore {
  public constructor(
    private readonly table: Table<KeyValueRow, string>,
    private readonly assertAvailable: () => void,
  ) {}

  public async get(key: string): Promise<JsonValue | undefined> {
    this.assertAvailable();
    const row = await this.table.get(key);
    return row === undefined ? undefined : parseJson(row.payload, 'key/value');
  }

  public async set(key: string, value: JsonValue): Promise<void> {
    this.assertAvailable();
    await write(() =>
      this.table.put({ key, payload: serialize(value, 'Value') }).then(() => undefined),
    );
  }

  public async delete(key: string): Promise<void> {
    this.assertAvailable();
    await write(() => this.table.delete(key));
  }

  public async clear(): Promise<void> {
    this.assertAvailable();
    await write(() => this.table.clear());
  }
}

/** Dexie implementation of ID-ordered record storage. */
export class DexieRecordStore<T extends StoredRecord> implements RecordStore<T> {
  public constructor(
    private readonly table: Table<RecordRow, string>,
    private readonly assertAvailable: () => void,
  ) {}

  public async put(record: T): Promise<void> {
    this.assertAvailable();
    if (record.id.length === 0) {
      throw new TypeError('Record id must not be empty.');
    }
    await write(() =>
      this.table.put({ id: record.id, payload: serialize(record, 'Record') }).then(() => undefined),
    );
  }

  public async get(id: string): Promise<T | undefined> {
    this.assertAvailable();
    const row = await this.table.get(id);
    return row === undefined ? undefined : (parseRecord(row.payload) as T);
  }

  public async delete(id: string): Promise<void> {
    this.assertAvailable();
    await write(() => this.table.delete(id));
  }

  public async list(options: PageOptions = {}): Promise<Page<T>> {
    this.assertAvailable();
    const limit = pageLimit(options.limit);
    const afterId = decodeCursor(options.cursor);
    const collection =
      afterId === undefined ? this.table.orderBy('id') : this.table.where('id').above(afterId);
    const rows = await collection.limit(limit + 1).toArray();
    const hasNext = rows.length > limit;
    const pageRows = rows.slice(0, limit);
    const last = pageRows.at(-1);
    return {
      items: pageRows.map((row) => parseRecord(row.payload) as T),
      nextCursor: hasNext && last !== undefined ? encodeCursor(last.id) : null,
    };
  }
}

/** Dexie implementation of portable schema metadata. */
export class DexieSchemaMetadataStore implements SchemaMetadataStore {
  public constructor(
    private readonly table: Table<SchemaRow, string>,
    private readonly assertAvailable: () => void,
  ) {}

  public async getSchemaMeta(): Promise<SchemaMeta | undefined> {
    this.assertAvailable();
    const row = await this.table.get('schema');
    return row === undefined ? undefined : { name: row.name, version: row.version };
  }

  public async setSchemaMeta(metadata: SchemaMeta): Promise<void> {
    this.assertAvailable();
    validateSchemaMeta(metadata);
    await write(() =>
      this.table
        .put({ key: 'schema', name: metadata.name, version: metadata.version })
        .then(() => undefined),
    );
  }
}

class DexieTransaction<T extends StoredRecord> implements ReentrantStorageTransaction<T> {
  public readonly keyValues: DexieKeyValueStore;
  public readonly records: DexieRecordStore<T>;
  public readonly schemaMetadata: DexieSchemaMetadataStore;
  private active = true;

  public constructor(database: BaukitDexieDatabase) {
    const assertActive = (): void => {
      if (!this.active) {
        throw new StorageError('storage_closed', 'The transaction context is no longer active.');
      }
    };
    this.keyValues = new DexieKeyValueStore(database.keyValues, assertActive);
    this.records = new DexieRecordStore<T>(database.records, assertActive);
    this.schemaMetadata = new DexieSchemaMetadataStore(database.schemaMetadata, assertActive);
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

export interface OpenDexieStoreOptions {
  readonly indexedDB?: DexieOptions['indexedDB'];
  readonly IDBKeyRange?: DexieOptions['IDBKeyRange'];
}

/** Complete Dexie 4.x adapter for Baukit's base storage contracts. */
export class DexieStore<T extends StoredRecord> implements TransactionalStorageStore<T> {
  public readonly keyValues: DexieKeyValueStore;
  public readonly records: DexieRecordStore<T>;
  public readonly schemaMetadata: DexieSchemaMetadataStore;
  private readonly lifecycle: Lifecycle = { status: 'open' };
  private transactionTail: Promise<void> = Promise.resolve();

  private constructor(private readonly database: BaukitDexieDatabase) {
    const assertAvailable = (): void => {
      assertOpen(this.lifecycle);
    };
    this.keyValues = new DexieKeyValueStore(database.keyValues, assertAvailable);
    this.records = new DexieRecordStore<T>(database.records, assertAvailable);
    this.schemaMetadata = new DexieSchemaMetadataStore(database.schemaMetadata, assertAvailable);
  }

  public static async open<TRecord extends StoredRecord>(
    databaseName: string,
    options: OpenDexieStoreOptions = {},
  ): Promise<DexieStore<TRecord>> {
    if (databaseName.length === 0) {
      throw new TypeError('Database name must not be empty.');
    }
    const dexieOptions: DexieOptions = {
      ...(options.indexedDB === undefined ? {} : { indexedDB: options.indexedDB }),
      ...(options.IDBKeyRange === undefined ? {} : { IDBKeyRange: options.IDBKeyRange }),
    };
    const database = new BaukitDexieDatabase(databaseName, dexieOptions);
    try {
      await database.open();
      return new DexieStore<TRecord>(database);
    } catch (cause) {
      database.close();
      throw normalizeStorageError(cause);
    }
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

    let transaction: DexieTransaction<T> | undefined;
    try {
      assertOpen(this.lifecycle);
      return await this.database.transaction(
        'rw!',
        [this.database.keyValues, this.database.records, this.database.schemaMetadata],
        async () => {
          transaction = new DexieTransaction<T>(this.database);
          return operation(transaction);
        },
      );
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
    this.database.close();
    this.lifecycle.status = 'closed';
  }
}

/** Opens a Dexie-backed store and closes it if initialization fails. */
export async function openDexieStore<T extends StoredRecord>(
  databaseName: string,
  options: OpenDexieStoreOptions = {},
): Promise<DexieStore<T>> {
  return DexieStore.open<T>(databaseName, options);
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

function decodeCursor(cursor: string | null | undefined): string | undefined {
  if (cursor === undefined || cursor === null) {
    return undefined;
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
