import { StorageError } from '@baukit/data-contracts';
import type { SQLiteDatabase } from 'expo-sqlite';

import { queueForFile } from './operation-queue.js';
import type { SQLiteConnection, StatementScope } from './statements.js';

export type SQLiteFileConnection = SQLiteConnection & Pick<SQLiteDatabase, 'databasePath'>;

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

/** Runs every statement on the queue shared by all adapter work against the database file. */
export function fileScope(
  database: SQLiteFileConnection,
  assertAvailable: () => void,
): StatementScope {
  return {
    assertAvailable,
    run: (statement) => queueForFile(database.databasePath).run(() => statement(database)),
  };
}

/** Statements on an open transaction's connection, rejected once the transaction has settled. */
export class TransactionScope implements StatementScope {
  private active = true;

  public constructor(private readonly connection: SQLiteConnection) {}

  public assertAvailable(): void {
    if (!this.active) {
      throw new StorageError('storage_closed', 'The transaction context is no longer active.');
    }
  }

  public run<TResult>(
    statement: (connection: SQLiteConnection) => Promise<TResult>,
  ): Promise<TResult> {
    return statement(this.connection);
  }

  public finish(): void {
    this.active = false;
  }
}

/**
 * Queues work on the database file and closes the handle after all accepted work, when it owns
 * the handle. Every `close()` caller gets the same promise.
 */
export class QueuedDatabase {
  private closeResult: Promise<void> | undefined;

  public constructor(
    public readonly database: ExpoSqliteDatabase,
    private readonly ownsDatabase: boolean,
  ) {}

  public enqueue<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    return queueForFile(this.database.databasePath).run(operation);
  }

  public close(): Promise<void> {
    this.closeResult ??= this.enqueue(() => this.closeOwnedDatabase());
    return this.closeResult;
  }

  public assertOpen(): void {
    if (this.closeResult !== undefined) {
      throw new StorageError('storage_closed', 'The storage adapter is closed.');
    }
  }

  private async closeOwnedDatabase(): Promise<void> {
    if (this.ownsDatabase) {
      await this.database.closeAsync();
    }
  }
}
