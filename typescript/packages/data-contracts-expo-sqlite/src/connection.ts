import type { SQLiteBindParams, SQLiteRunResult, SQLiteVariadicBindParams } from 'expo-sqlite';

import { type ExpoSqliteDatabase, QueuedDatabase, TransactionScope } from './queued-database.js';
import type { SQLiteConnection, StatementScope } from './statements.js';

type BindArguments = [SQLiteBindParams] | SQLiteVariadicBindParams;

const ENFORCE_FOREIGN_KEYS = 'PRAGMA foreign_keys = ON';
const BEGIN = 'BEGIN IMMEDIATE';
const COMMIT = 'COMMIT';
const ROLLBACK = 'ROLLBACK';

/**
 * Raw SQL statements. Parameters bind as `expo-sqlite` binds them: variadic values, one array, or
 * one object whose keys keep their `$`, `:`, or `@` prefix.
 */
export interface SqliteStatements {
  /** Runs a script of one or more statements without parameters. */
  exec(source: string): Promise<void>;
  run(source: string, params: SQLiteBindParams): Promise<SQLiteRunResult>;
  run(source: string, ...params: SQLiteVariadicBindParams): Promise<SQLiteRunResult>;
  /** Resolves to the first row, or `undefined` when there is none. */
  get<TRow>(source: string, params: SQLiteBindParams): Promise<TRow | undefined>;
  get<TRow>(source: string, ...params: SQLiteVariadicBindParams): Promise<TRow | undefined>;
  all<TRow>(source: string, params: SQLiteBindParams): Promise<TRow[]>;
  all<TRow>(source: string, ...params: SQLiteVariadicBindParams): Promise<TRow[]>;
}

/** Statements inside one open transaction. They reject once the transaction has settled. */
export interface SqliteTransaction extends SqliteStatements {
  /** Always rejects: a transaction never joins or nests another one. */
  transaction(work: (transaction: SqliteTransaction) => unknown): Promise<never>;
}

export interface ExpoSqliteConnectionOptions {
  /** Close the supplied database when this connection closes. Defaults to false. */
  readonly closeDatabase?: boolean;
}

class ScopedStatements implements SqliteStatements {
  public constructor(protected readonly scope: StatementScope) {}

  public async exec(source: string): Promise<void> {
    this.scope.assertAvailable();
    await this.scope.run((connection) => connection.execAsync(source));
  }

  public run(source: string, params: SQLiteBindParams): Promise<SQLiteRunResult>;
  public run(source: string, ...params: SQLiteVariadicBindParams): Promise<SQLiteRunResult>;
  public async run(source: string, ...params: BindArguments): Promise<SQLiteRunResult> {
    this.scope.assertAvailable();
    return this.scope.run((connection) => connection.runAsync(source, ...variadic(params)));
  }

  public get<TRow>(source: string, params: SQLiteBindParams): Promise<TRow | undefined>;
  public get<TRow>(source: string, ...params: SQLiteVariadicBindParams): Promise<TRow | undefined>;
  public async get<TRow>(source: string, ...params: BindArguments): Promise<TRow | undefined> {
    this.scope.assertAvailable();
    const row = await this.scope.run((connection) =>
      connection.getFirstAsync<TRow>(source, ...variadic(params)),
    );
    return row ?? undefined;
  }

  public all<TRow>(source: string, params: SQLiteBindParams): Promise<TRow[]>;
  public all<TRow>(source: string, ...params: SQLiteVariadicBindParams): Promise<TRow[]>;
  public async all<TRow>(source: string, ...params: BindArguments): Promise<TRow[]> {
    this.scope.assertAvailable();
    return this.scope.run((connection) =>
      connection.getAllAsync<TRow>(source, ...variadic(params)),
    );
  }
}

class ConnectionTransaction extends ScopedStatements implements SqliteTransaction {
  public transaction(): Promise<never> {
    return Promise.reject(
      new TypeError(
        'A SQLite transaction cannot start another transaction. Run the statements on the current transaction instead.',
      ),
    );
  }
}

/** The root scope: queued on the database file, with foreign keys enabled before any work. */
class ConnectionScope implements StatementScope {
  private foreignKeysEnforced = false;

  public constructor(public readonly queued: QueuedDatabase) {}

  public assertAvailable(): void {
    this.queued.assertOpen();
  }

  public run<TResult>(
    statement: (connection: SQLiteConnection) => Promise<TResult>,
  ): Promise<TResult> {
    return this.queued.enqueue(async () => {
      await this.enforceForeignKeys();
      return statement(this.queued.database);
    });
  }

  private async enforceForeignKeys(): Promise<void> {
    if (this.foreignKeysEnforced) {
      return;
    }
    await this.queued.database.execAsync(ENFORCE_FOREIGN_KEYS);
    this.foreignKeysEnforced = true;
  }
}

/**
 * A raw SQL connection over an Expo SQLite database. Statements, transactions, and `close()` run
 * one at a time in call order on the queue that every adapter store on the same database file
 * shares, so a root statement never runs inside another caller's transaction. Transactions run
 * `BEGIN IMMEDIATE` on the supplied handle, and the connection enables foreign keys before its
 * first statement, so they stay enforced inside transactions.
 */
export class ExpoSqliteConnection extends ScopedStatements {
  private readonly root: ConnectionScope;

  public constructor(database: ExpoSqliteDatabase, options: ExpoSqliteConnectionOptions = {}) {
    const root = new ConnectionScope(new QueuedDatabase(database, options.closeDatabase === true));
    super(root);
    this.root = root;
  }

  /**
   * Runs `work` in one transaction that commits when it resolves and rolls back when it rejects.
   * Use only the transaction it receives: a root call on this file from inside `work` waits for
   * the transaction, which waits for `work`, and neither finishes.
   */
  public async transaction<TResult>(
    work: (transaction: SqliteTransaction) => Promise<TResult> | TResult,
  ): Promise<TResult> {
    this.root.assertAvailable();
    return this.root.run((connection) => runTransaction(connection, work));
  }

  /** Rejects new work, waits for accepted work, and settles every caller with one result. */
  public close(): Promise<void> {
    return this.root.queued.close();
  }
}

async function runTransaction<TResult>(
  connection: SQLiteConnection,
  work: (transaction: SqliteTransaction) => Promise<TResult> | TResult,
): Promise<TResult> {
  await connection.execAsync(BEGIN);
  const scope = new TransactionScope(connection);
  try {
    const result = await work(new ConnectionTransaction(scope));
    await connection.execAsync(COMMIT);
    return result;
  } catch (cause) {
    await rollback(connection);
    throw cause;
  } finally {
    scope.finish();
  }
}

/** Keeps the original failure when SQLite has already rolled the transaction back. */
async function rollback(connection: SQLiteConnection): Promise<void> {
  try {
    await connection.execAsync(ROLLBACK);
  } catch {
    return;
  }
}

function variadic(params: BindArguments): SQLiteVariadicBindParams {
  return params as SQLiteVariadicBindParams;
}
