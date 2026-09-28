import {
  type KeyValueStore,
  type RecordStore,
  type ReentrantStorageTransaction,
  type SchemaMetadataStore,
  type StoredRecord,
  type TransactionalStorageStore,
  normalizeStorageError,
} from '@baukit/data-contracts';

import {
  type ExpoSqliteDatabase,
  QueuedDatabase,
  type SQLiteFileConnection,
  TransactionScope,
  fileScope,
} from './queued-database.js';
import {
  CREATE_ADAPTER_TABLES,
  KeyValueStatements,
  RecordStatements,
  type SQLiteConnection,
  SchemaMetadataStatements,
  type StatementScope,
  write,
} from './statements.js';

export type { ExpoSqliteDatabase } from './queued-database.js';
export {
  ExpoSqliteConnection,
  type ExpoSqliteConnectionOptions,
  type SqliteStatements,
  type SqliteTransaction,
} from './connection.js';

const alwaysAvailable = (): void => undefined;

/**
 * A namespaced Expo SQLite implementation of Baukit's provider-neutral RecordStore.
 * Statements share one queue with every adapter store that uses the same database file.
 */
export class SqliteRecordStore<T extends StoredRecord> extends RecordStatements<T> {
  public constructor(database: SQLiteFileConnection, namespace: string) {
    super(fileScope(database, alwaysAvailable), namespace);
  }
}

/** Namespaced key/value storage ordered with every adapter store on the same database file. */
export class SqliteKeyValueStore extends KeyValueStatements {
  public constructor(database: SQLiteFileConnection, namespace: string) {
    super(fileScope(database, alwaysAvailable), namespace);
  }
}

/** Namespaced schema metadata ordered with every adapter store on the same database file. */
export class SqliteSchemaMetadataStore extends SchemaMetadataStatements {
  public constructor(database: SQLiteFileConnection, namespace: string) {
    super(fileScope(database, alwaysAvailable), namespace);
  }
}

class ExpoSqliteTransaction<T extends StoredRecord> implements ReentrantStorageTransaction<T> {
  public readonly keyValues: KeyValueStatements;
  public readonly records: RecordStatements<T>;
  public readonly schemaMetadata: SchemaMetadataStatements;
  private readonly scope: TransactionScope;

  public constructor(connection: SQLiteConnection, namespace: string) {
    this.scope = new TransactionScope(connection);
    this.keyValues = new KeyValueStatements(this.scope, namespace);
    this.records = new RecordStatements<T>(this.scope, namespace);
    this.schemaMetadata = new SchemaMetadataStatements(this.scope, namespace);
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    this.scope.assertAvailable();
    try {
      return await operation(this);
    } catch (cause) {
      throw normalizeStorageError(cause);
    }
  }

  public finish(): void {
    this.scope.finish();
  }
}

export interface ExpoSqliteStoreOptions {
  /** Close the supplied database when this adapter closes. Defaults to false. */
  readonly closeDatabase?: boolean;
}

/**
 * Complete namespaced adapter using Expo SQLite exclusive write transactions.
 * Root statements, transactions, and close run one at a time in call order, shared with every
 * adapter store on the same database file.
 */
export class ExpoSqliteStore<T extends StoredRecord> implements TransactionalStorageStore<T> {
  public readonly keyValues: KeyValueStore;
  public readonly records: RecordStore<T>;
  public readonly schemaMetadata: SchemaMetadataStore;
  private readonly root: StatementScope;
  private readonly queued: QueuedDatabase;

  public constructor(
    database: ExpoSqliteDatabase,
    private readonly namespace: string,
    options: ExpoSqliteStoreOptions = {},
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Storage namespace must not be empty.');
    }
    this.queued = new QueuedDatabase(database, options.closeDatabase === true);
    this.root = fileScope(database, () => {
      this.queued.assertOpen();
    });
    this.keyValues = new KeyValueStatements(this.root, namespace);
    this.records = new RecordStatements<T>(this.root, namespace);
    this.schemaMetadata = new SchemaMetadataStatements(this.root, namespace);
  }

  public async initialize(): Promise<void> {
    this.queued.assertOpen();
    await write(() => this.root.run((connection) => connection.execAsync(CREATE_ADAPTER_TABLES)));
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    this.queued.assertOpen();
    try {
      return await this.queued.enqueue(() => this.runExclusive(operation));
    } catch (cause) {
      throw normalizeStorageError(cause);
    }
  }

  /** Rejects new work, waits for accepted work, and settles every caller with one result. */
  public close(): Promise<void> {
    return this.queued.close();
  }

  private async runExclusive<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    let transaction: ExpoSqliteTransaction<T> | undefined;
    let outcome: { value: TResult } | undefined;
    try {
      await this.queued.database.withExclusiveTransactionAsync(async (connection) => {
        transaction = new ExpoSqliteTransaction<T>(connection, this.namespace);
        outcome = { value: await operation(transaction) };
      });
    } finally {
      transaction?.finish();
    }
    if (outcome === undefined) {
      throw new Error('SQLite transaction completed without a callback result.');
    }
    return outcome.value;
  }
}
