import {
  type KeyValueStore,
  type RecordStore,
  type ReentrantStorageTransaction,
  type SchemaMetadataStore,
  StorageError,
  type StoredRecord,
  type TransactionalStorageStore,
  normalizeStorageError,
} from '@baukit/data-contracts';
import type { SQLiteDatabase } from 'expo-sqlite';

import { type OperationQueue, queueForFile } from './operation-queue.js';
import {
  CREATE_ADAPTER_TABLES,
  KeyValueStatements,
  RecordStatements,
  type SQLiteConnection,
  SchemaMetadataStatements,
  type StatementScope,
  write,
} from './statements.js';

type SQLiteFileConnection = SQLiteConnection & Pick<SQLiteDatabase, 'databasePath'>;

/**
 * The part of an Expo `SQLiteDatabase` this adapter calls. `NodeSqliteDatabase` from the
 * `./testing` entry point implements it for Node unit tests.
 */
export interface ExpoSqliteDatabase extends SQLiteFileConnection {
  closeAsync(): Promise<void>;
  withExclusiveTransactionAsync(
    task: (transaction: SQLiteConnection) => Promise<void>,
  ): Promise<void>;
}

const alwaysAvailable = (): void => undefined;

function fileScope(database: SQLiteFileConnection, assertAvailable: () => void): StatementScope {
  return {
    assertAvailable,
    run: (statement) => queueForFile(database.databasePath).run(() => statement(database)),
  };
}

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
  private active = true;

  public constructor(connection: SQLiteConnection, namespace: string) {
    const scope: StatementScope = {
      assertAvailable: () => {
        this.assertActive();
      },
      run: (statement) => statement(connection),
    };
    this.keyValues = new KeyValueStatements(scope, namespace);
    this.records = new RecordStatements<T>(scope, namespace);
    this.schemaMetadata = new SchemaMetadataStatements(scope, namespace);
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    this.assertActive();
    try {
      return await operation(this);
    } catch (cause) {
      throw normalizeStorageError(cause);
    }
  }

  public finish(): void {
    this.active = false;
  }

  private assertActive(): void {
    if (!this.active) {
      throw new StorageError('storage_closed', 'The transaction context is no longer active.');
    }
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
  private closeResult: Promise<void> | undefined;

  public constructor(
    private readonly database: ExpoSqliteDatabase,
    private readonly namespace: string,
    private readonly options: ExpoSqliteStoreOptions = {},
  ) {
    if (namespace.length === 0) {
      throw new TypeError('Storage namespace must not be empty.');
    }
    this.root = fileScope(database, () => {
      this.assertOpen();
    });
    this.keyValues = new KeyValueStatements(this.root, namespace);
    this.records = new RecordStatements<T>(this.root, namespace);
    this.schemaMetadata = new SchemaMetadataStatements(this.root, namespace);
  }

  public async initialize(): Promise<void> {
    this.assertOpen();
    await write(() => this.root.run((connection) => connection.execAsync(CREATE_ADAPTER_TABLES)));
  }

  public async withTransaction<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    this.assertOpen();
    try {
      return await this.queue().run(() => this.runExclusive(operation));
    } catch (cause) {
      throw normalizeStorageError(cause);
    }
  }

  /** Rejects new work, waits for accepted work, and settles every caller with one result. */
  public close(): Promise<void> {
    this.closeResult ??= this.queue().run(() => this.closeOwnedDatabase());
    return this.closeResult;
  }

  private async runExclusive<TResult>(
    operation: (context: ReentrantStorageTransaction<T>) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    let transaction: ExpoSqliteTransaction<T> | undefined;
    let outcome: { value: TResult } | undefined;
    try {
      await this.database.withExclusiveTransactionAsync(async (connection) => {
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

  private async closeOwnedDatabase(): Promise<void> {
    if (this.options.closeDatabase === true) {
      await this.database.closeAsync();
    }
  }

  private queue(): OperationQueue {
    return queueForFile(this.database.databasePath);
  }

  private assertOpen(): void {
    if (this.closeResult !== undefined) {
      throw new StorageError('storage_closed', 'The storage adapter is closed.');
    }
  }
}
