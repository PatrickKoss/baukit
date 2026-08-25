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
  type SchemaMigrationHook,
  type StoredRecord,
  StorageError,
  type TransactionalStorageStore,
  normalizeStorageError,
} from './contracts.js';

const CURSOR_PREFIX = 'bk1:';

function cloneUnknown(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item: unknown) => cloneUnknown(item));
  }
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, item]) => [
        key,
        cloneUnknown(item),
      ]),
    );
  }
  return value;
}

function clone<T>(value: T): T {
  return cloneUnknown(value) as T;
}

function compareIds(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function pageLimit(options?: PageOptions): number {
  const limit = options?.limit ?? DEFAULT_PAGE_SIZE;
  if (!Number.isInteger(limit) || limit < 1 || limit > MAX_PAGE_SIZE) {
    throw new RangeError(`Page limit must be an integer from 1 to ${String(MAX_PAGE_SIZE)}.`);
  }
  return limit;
}

function encodeCursor(id: string): string {
  return `${CURSOR_PREFIX}${encodeURIComponent(id)}`;
}

function decodeCursor(cursor: string): string {
  if (!cursor.startsWith(CURSOR_PREFIX)) {
    throw new TypeError('Invalid record cursor.');
  }
  try {
    return decodeURIComponent(cursor.slice(CURSOR_PREFIX.length));
  } catch {
    throw new TypeError('Invalid record cursor.');
  }
}

interface KeyValueState {
  values: Map<string, JsonValue>;
}

interface RecordState<T extends StoredRecord> {
  values: Map<string, T>;
}

interface MetadataState {
  value: SchemaMeta | undefined;
}

interface MemoryState<T extends StoredRecord> {
  keyValues: KeyValueState;
  records: RecordState<T>;
  schemaMetadata: MetadataState;
}

interface MemoryLifecycle {
  status: 'closed' | 'closing' | 'open';
}

function assertStoreOpen(lifecycle: MemoryLifecycle): void {
  if (lifecycle.status !== 'open') {
    throw new StorageError('storage_closed', 'The storage adapter is closed.');
  }
}

function emptyState<T extends StoredRecord>(): MemoryState<T> {
  return {
    keyValues: { values: new Map() },
    records: { values: new Map() },
    schemaMetadata: { value: undefined },
  };
}

function copyState<T extends StoredRecord>(state: MemoryState<T>): MemoryState<T> {
  return {
    keyValues: {
      values: new Map([...state.keyValues.values].map(([key, value]) => [key, clone(value)])),
    },
    records: {
      values: new Map([...state.records.values].map(([key, value]) => [key, clone(value)])),
    },
    schemaMetadata: {
      value:
        state.schemaMetadata.value === undefined ? undefined : clone(state.schemaMetadata.value),
    },
  };
}

/** Dependency-free reference key/value adapter. */
export class InMemoryKeyValueStore implements KeyValueStore {
  public constructor(
    private readonly state: KeyValueState = { values: new Map() },
    private readonly assertAvailable: () => void = () => undefined,
  ) {}

  public get(key: string): Promise<JsonValue | undefined> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      const value = this.state.values.get(key);
      return value === undefined ? undefined : clone(value);
    });
  }

  public set(key: string, value: JsonValue): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      this.state.values.set(key, clone(value));
    });
  }

  public delete(key: string): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      this.state.values.delete(key);
    });
  }

  public clear(): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      this.state.values.clear();
    });
  }
}

/** Dependency-free reference record adapter using ID-based keyset cursors. */
export class InMemoryRecordStore<T extends StoredRecord> implements RecordStore<T> {
  public constructor(
    private readonly state: RecordState<T> = { values: new Map() },
    private readonly assertAvailable: () => void = () => undefined,
  ) {}

  public put(record: T): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      if (record.id.length === 0) {
        throw new TypeError('Record id must not be empty.');
      }
      this.state.values.set(record.id, clone(record));
    });
  }

  public get(id: string): Promise<T | undefined> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      const record = this.state.values.get(id);
      return record === undefined ? undefined : clone(record);
    });
  }

  public delete(id: string): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      this.state.values.delete(id);
    });
  }

  public list(options?: PageOptions): Promise<Page<T>> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      const limit = pageLimit(options);
      const afterId =
        options?.cursor === undefined || options.cursor === null
          ? undefined
          : decodeCursor(options.cursor);
      const ordered = [...this.state.values.values()]
        .filter((record) => afterId === undefined || compareIds(record.id, afterId) > 0)
        .sort((left, right) => compareIds(left.id, right.id));
      const pageItems = ordered.slice(0, limit);
      const last = pageItems.at(-1);

      return {
        items: clone(pageItems),
        nextCursor: ordered.length > limit && last !== undefined ? encodeCursor(last.id) : null,
      };
    });
  }
}

/** Dependency-free reference schema metadata adapter. */
export class InMemorySchemaMetadataStore implements SchemaMetadataStore {
  public constructor(
    private readonly state: MetadataState = { value: undefined },
    private readonly assertAvailable: () => void = () => undefined,
  ) {}

  public getSchemaMeta(): Promise<SchemaMeta | undefined> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      return this.state.value === undefined ? undefined : clone(this.state.value);
    });
  }

  public setSchemaMeta(metadata: SchemaMeta): Promise<void> {
    return Promise.resolve().then(() => {
      this.assertAvailable();
      if (metadata.name.length === 0) {
        throw new TypeError('Schema name must not be empty.');
      }
      if (!Number.isInteger(metadata.version) || metadata.version < 0) {
        throw new TypeError('Schema version must be a non-negative integer.');
      }
      this.state.value = clone(metadata);
    });
  }
}

/** A transaction-scoped in-memory view. It must not be retained after the callback. */
export class InMemoryTransaction<T extends StoredRecord> implements ReentrantStorageTransaction<T> {
  public readonly keyValues: InMemoryKeyValueStore;
  public readonly records: InMemoryRecordStore<T>;
  public readonly schemaMetadata: InMemorySchemaMetadataStore;

  private active = true;

  public constructor(
    protected readonly state: MemoryState<T>,
    assertAvailable?: () => void,
  ) {
    const assertTransactionActive =
      assertAvailable ??
      (() => {
        if (!this.active) {
          throw new StorageError('storage_closed', 'The transaction context is no longer active.');
        }
      });
    this.keyValues = new InMemoryKeyValueStore(state.keyValues, assertTransactionActive);
    this.records = new InMemoryRecordStore(state.records, assertTransactionActive);
    this.schemaMetadata = new InMemorySchemaMetadataStore(
      state.schemaMetadata,
      assertTransactionActive,
    );
  }

  public withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    return Promise.resolve().then(() => {
      if (!this.active) {
        throw new StorageError('storage_closed', 'The transaction context is no longer active.');
      }
      return operation(this);
    });
  }

  public finish(): void {
    this.active = false;
  }
}

/** Composite reference adapter with snapshot-backed atomic transactions. */
export class InMemoryStore<T extends StoredRecord>
  extends InMemoryTransaction<T>
  implements TransactionalStorageStore<T>, SchemaMigrationHook
{
  private transactionTail: Promise<void> = Promise.resolve();
  private readonly lifecycle: MemoryLifecycle;

  public constructor(state: MemoryState<T> = emptyState()) {
    const lifecycle: MemoryLifecycle = { status: 'open' };
    super(state, () => {
      assertStoreOpen(lifecycle);
    });
    this.lifecycle = lifecycle;
  }

  public override async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    assertStoreOpen(this.lifecycle);
    const previous = this.transactionTail;
    let release = (): void => undefined;
    this.transactionTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;

    let transaction: InMemoryTransaction<T> | undefined;
    try {
      assertStoreOpen(this.lifecycle);
      const pending = copyState(this.state);
      transaction = new InMemoryTransaction(pending);
      const result = await operation(transaction);
      this.state.keyValues.values = pending.keyValues.values;
      this.state.records.values = pending.records.values;
      this.state.schemaMetadata.value = pending.schemaMetadata.value;
      return result;
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
    this.lifecycle.status = 'closed';
  }

  public async migrate(from: SchemaMeta, to: SchemaMeta): Promise<void> {
    const current = await this.schemaMetadata.getSchemaMeta();
    if (current?.name !== from.name || current.version !== from.version) {
      throw new Error('Current schema metadata does not match the migration source.');
    }
    if (from.name !== to.name) {
      throw new Error('A migration cannot change the schema name.');
    }
    if (to.version <= from.version) {
      throw new Error('A migration target version must be greater than its source.');
    }
    await this.schemaMetadata.setSchemaMeta(to);
  }
}

/** Named persistent backing for exercising close/reopen identity transitions in memory. */
export class InMemoryStorePool<T extends StoredRecord> {
  private readonly databases = new Map<string, MemoryState<T>>();

  public open(storeName: string): InMemoryStore<T> {
    if (storeName.trim().length === 0) {
      throw new TypeError('Store name must not be empty.');
    }
    let state = this.databases.get(storeName);
    if (state === undefined) {
      state = emptyState();
      this.databases.set(storeName, state);
    }
    return new InMemoryStore(state);
  }
}
