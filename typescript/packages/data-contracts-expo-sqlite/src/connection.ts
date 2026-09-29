import { StorageError } from '@baukit/data-contracts';
import type { SQLiteBindParams, SQLiteRunResult, SQLiteVariadicBindParams } from 'expo-sqlite';

import { type ExpoSqliteDatabase, QueuedDatabase } from './queued-database.js';
import type { SQLiteConnection, StatementScope } from './statements.js';

type BindArguments = [SQLiteBindParams] | SQLiteVariadicBindParams;

const ENFORCE_FOREIGN_KEYS = 'PRAGMA foreign_keys = ON';
const ROOT_DEPTH = 0;
const SAVEPOINT_PREFIX = 'baukit_nested_';

/** The statements that open, commit, and roll back one transaction level. */
interface Boundary {
  readonly begin: string;
  readonly commit: string;
  readonly rollback: string;
}

const ROOT_BOUNDARY: Boundary = {
  begin: 'BEGIN IMMEDIATE',
  commit: 'COMMIT',
  rollback: 'ROLLBACK',
};

type TransactionWork<TResult> = (transaction: SqliteTransaction) => Promise<TResult> | TResult;

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

/**
 * Statements inside one open transaction. They reject with `storage_closed` once the transaction
 * has settled, and with a `TypeError` while a nested transaction started from it is open.
 */
export interface SqliteTransaction extends SqliteStatements {
  /**
   * Runs `work` in a savepoint inside this transaction, without waiting on the file queue. The
   * savepoint is released when `work` resolves and rolled back when it rejects; the rejection
   * then reaches the caller, and the enclosing transaction commits only if the caller catches it.
   */
  transaction<TResult>(work: TransactionWork<TResult>): Promise<TResult>;
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

/** One open transaction or savepoint. Only the innermost open level accepts work. */
class TransactionLevel implements StatementScope {
  private active = true;
  private nestedOpen = false;

  public constructor(
    private readonly connection: SQLiteConnection,
    private readonly depth: number,
  ) {}

  public assertAvailable(): void {
    if (!this.active) {
      throw new StorageError('storage_closed', 'The transaction context is no longer active.');
    }
    if (this.nestedOpen) {
      throw new TypeError(
        'A nested SQLite transaction is open. Use the nested transaction until it settles.',
      );
    }
  }

  public run<TResult>(
    statement: (connection: SQLiteConnection) => Promise<TResult>,
  ): Promise<TResult> {
    return statement(this.connection);
  }

  public async nest<TResult>(work: TransactionWork<TResult>): Promise<TResult> {
    this.assertAvailable();
    this.nestedOpen = true;
    try {
      return await runLevel(this.connection, this.depth + 1, work);
    } finally {
      this.nestedOpen = false;
    }
  }

  public finish(): void {
    this.active = false;
  }
}

class ConnectionTransaction extends ScopedStatements implements SqliteTransaction {
  public constructor(private readonly level: TransactionLevel) {
    super(level);
  }

  public transaction<TResult>(work: TransactionWork<TResult>): Promise<TResult> {
    return this.level.nest(work);
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
   * Use only the transaction it receives, and nest through its `transaction()`: a root call on
   * this file from inside `work` waits for the transaction, which waits for `work`, and neither
   * finishes.
   */
  public async transaction<TResult>(work: TransactionWork<TResult>): Promise<TResult> {
    this.root.assertAvailable();
    return this.root.run((connection) => runLevel(connection, ROOT_DEPTH, work));
  }

  /** Rejects new work, waits for accepted work, and settles every caller with one result. */
  public close(): Promise<void> {
    return this.root.queued.close();
  }
}

function boundaryAt(depth: number): Boundary {
  if (depth === ROOT_DEPTH) {
    return ROOT_BOUNDARY;
  }
  const name = `${SAVEPOINT_PREFIX}${String(depth)}`;
  return {
    begin: `SAVEPOINT ${name}`,
    commit: `RELEASE ${name}`,
    rollback: `ROLLBACK TO ${name}; RELEASE ${name}`,
  };
}

async function runLevel<TResult>(
  connection: SQLiteConnection,
  depth: number,
  work: TransactionWork<TResult>,
): Promise<TResult> {
  const boundary = boundaryAt(depth);
  await connection.execAsync(boundary.begin);
  const level = new TransactionLevel(connection, depth);
  try {
    const result = await work(new ConnectionTransaction(level));
    await connection.execAsync(boundary.commit);
    return result;
  } catch (cause) {
    await rollback(connection, boundary);
    throw cause;
  } finally {
    level.finish();
  }
}

/** Keeps the original failure when SQLite has already rolled the transaction back. */
async function rollback(connection: SQLiteConnection, boundary: Boundary): Promise<void> {
  try {
    await connection.execAsync(boundary.rollback);
  } catch {
    return;
  }
}

function variadic(params: BindArguments): SQLiteVariadicBindParams {
  return params as SQLiteVariadicBindParams;
}
