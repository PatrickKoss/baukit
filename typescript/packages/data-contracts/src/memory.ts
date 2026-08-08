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
  type SchemaMigrationHook,
  type StorageTransaction,
  type StoredRecord,
  type Transaction,
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
  public constructor(private readonly state: KeyValueState = { values: new Map() }) {}

  public get(key: string): Promise<JsonValue | undefined> {
    const value = this.state.values.get(key);
    return Promise.resolve(value === undefined ? undefined : clone(value));
  }

  public set(key: string, value: JsonValue): Promise<void> {
    this.state.values.set(key, clone(value));
    return Promise.resolve();
  }

  public delete(key: string): Promise<void> {
    this.state.values.delete(key);
    return Promise.resolve();
  }

  public clear(): Promise<void> {
    this.state.values.clear();
    return Promise.resolve();
  }
}

/** Dependency-free reference record adapter using ID-based keyset cursors. */
export class InMemoryRecordStore<T extends StoredRecord> implements RecordStore<T> {
  public constructor(private readonly state: RecordState<T> = { values: new Map() }) {}

  public put(record: T): Promise<void> {
    if (record.id.length === 0) {
      throw new TypeError('Record id must not be empty.');
    }
    this.state.values.set(record.id, clone(record));
    return Promise.resolve();
  }

  public get(id: string): Promise<T | undefined> {
    const record = this.state.values.get(id);
    return Promise.resolve(record === undefined ? undefined : clone(record));
  }

  public delete(id: string): Promise<void> {
    this.state.values.delete(id);
    return Promise.resolve();
  }

  public list(options?: PageOptions): Promise<Page<T>> {
    return Promise.resolve().then(() => {
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
  public constructor(private readonly state: MetadataState = { value: undefined }) {}

  public getSchemaMeta(): Promise<SchemaMeta | undefined> {
    return Promise.resolve(this.state.value === undefined ? undefined : clone(this.state.value));
  }

  public setSchemaMeta(metadata: SchemaMeta): Promise<void> {
    if (metadata.name.length === 0) {
      throw new TypeError('Schema name must not be empty.');
    }
    if (!Number.isInteger(metadata.version) || metadata.version < 0) {
      throw new TypeError('Schema version must be a non-negative integer.');
    }
    this.state.value = clone(metadata);
    return Promise.resolve();
  }
}

/** A transaction-scoped in-memory view. It must not be retained after the callback. */
export class InMemoryTransaction<T extends StoredRecord> implements StorageTransaction<T> {
  public readonly keyValues: InMemoryKeyValueStore;
  public readonly records: InMemoryRecordStore<T>;
  public readonly schemaMetadata: InMemorySchemaMetadataStore;

  public constructor(protected readonly state: MemoryState<T>) {
    this.keyValues = new InMemoryKeyValueStore(state.keyValues);
    this.records = new InMemoryRecordStore(state.records);
    this.schemaMetadata = new InMemorySchemaMetadataStore(state.schemaMetadata);
  }
}

/** Composite reference adapter with snapshot-backed atomic transactions. */
export class InMemoryStore<T extends StoredRecord>
  extends InMemoryTransaction<T>
  implements Transaction<StorageTransaction<T>>, SchemaMigrationHook
{
  private transactionTail: Promise<void> = Promise.resolve();

  public constructor() {
    super(emptyState());
  }

  public async withTransaction<TResult>(
    operation: (context: StorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    const previous = this.transactionTail;
    let release = (): void => undefined;
    this.transactionTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;

    try {
      const pending = copyState(this.state);
      const result = await operation(new InMemoryTransaction(pending));
      this.state.keyValues.values = pending.keyValues.values;
      this.state.records.values = pending.records.values;
      this.state.schemaMetadata.value = pending.schemaMetadata.value;
      return result;
    } finally {
      release();
    }
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
