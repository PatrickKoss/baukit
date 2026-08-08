/** A value that can be represented without loss by JSON. */
export type JsonPrimitive = boolean | number | string | null;

/** A recursively JSON-serializable value. */
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

/** A provider-neutral asynchronous string-keyed store. */
export interface KeyValueStore {
  /** Returns `undefined` when the key does not exist. */
  get(key: string): Promise<JsonValue | undefined>;
  set(key: string, value: JsonValue): Promise<void>;
  /** Deletes a key if present. Deleting a missing key is a no-op. */
  delete(key: string): Promise<void>;
  clear(): Promise<void>;
}

/** Records use an immutable string ID as their stable ordering and lookup key. */
export interface StoredRecord {
  readonly id: string;
}

/** An opaque continuation token returned by a previous `list` call. */
export type Cursor = string;

export interface PageOptions {
  /** Omit (or pass `null`) to begin at the first record. */
  readonly cursor?: Cursor | null;
  /** Must be an integer between 1 and `MAX_PAGE_SIZE`, inclusive. */
  readonly limit?: number;
}

/** A cursor page. A null cursor means the page is terminal. */
export interface Page<T> {
  readonly items: readonly T[];
  readonly nextCursor: Cursor | null;
}

export const DEFAULT_PAGE_SIZE = 50;
export const MAX_PAGE_SIZE = 100;

/**
 * A record store ordered by immutable `id`, ascending by JavaScript string
 * comparison. Cursors are keyset positions: inserts before a returned cursor
 * never shift or duplicate later pages.
 */
export interface RecordStore<T extends StoredRecord> {
  /** Inserts or replaces the record with the same ID. */
  put(record: T): Promise<void>;
  /** Returns `undefined` when the ID does not exist. */
  get(id: string): Promise<T | undefined>;
  /** Deletes a record if present. Deleting a missing ID is a no-op. */
  delete(id: string): Promise<void>;
  list(options?: PageOptions): Promise<Page<T>>;
}

/** Portable schema identity persisted alongside adapter data. */
export interface SchemaMeta {
  readonly name: string;
  /** A non-negative integer incremented for every schema change. */
  readonly version: number;
}

export interface SchemaMetadataStore {
  /** Returns `undefined` before metadata is initialized. */
  getSchemaMeta(): Promise<SchemaMeta | undefined>;
  setSchemaMeta(metadata: SchemaMeta): Promise<void>;
}

/** A product-provided hook used to move between two schema identities. */
export interface SchemaMigrationHook {
  migrate(from: SchemaMeta, to: SchemaMeta): Promise<void>;
}

export type MaybePromise<T> = Promise<T> | T;

/**
 * Executes work against a transaction-scoped context. If the callback throws
 * or rejects, none of its writes become visible. Its return value is preserved.
 */
export interface Transaction<TContext> {
  withTransaction<TResult>(
    operation: (context: TContext) => MaybePromise<TResult>,
  ): Promise<TResult>;
}

/** The standard transaction view used by composite storage adapters. */
export interface StorageTransaction<T extends StoredRecord> {
  readonly keyValues: KeyValueStore;
  readonly records: RecordStore<T>;
  readonly schemaMetadata: SchemaMetadataStore;
}
