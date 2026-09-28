import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync, type SQLInputValue, type StatementSync } from 'node:sqlite';

import type { SQLiteBindParams, SQLiteRunResult, SQLiteVariadicBindParams } from 'expo-sqlite';

import type { ExpoSqliteDatabase } from './queued-database.js';

type NamedValues = Record<string, SQLInputValue>;

interface PreparedCall {
  readonly statement: StatementSync;
  readonly named: NamedValues;
  readonly positional: SQLInputValue[];
}

interface DatabaseFile {
  readonly path: string;
  readonly ownedDirectory: string | undefined;
}

const MEMORY_DATABASE_PATH = ':memory:';
const TEMPORARY_DIRECTORY_PREFIX = 'baukit-sqlite-';
const TEMPORARY_DATABASE_FILE = 'database.sqlite';

/**
 * Expo opens every native connection with SQLite's compile-time defaults: foreign keys off,
 * double-quoted string literals accepted, and no busy timeout.
 */
const EXPO_CONNECTION_OPTIONS = {
  enableForeignKeyConstraints: false,
  enableDoubleQuotedStringLiterals: true,
  timeout: 0,
} as const;

/** Statement methods shared by the root handle and an exclusive transaction connection. */
export class NodeSqliteConnection {
  public constructor(protected readonly connection: DatabaseSync) {}

  public async execAsync(source: string): Promise<void> {
    await nextTurn();
    this.connection.exec(source);
  }

  public runAsync(source: string, params: SQLiteBindParams): Promise<SQLiteRunResult>;
  public runAsync(source: string, ...params: SQLiteVariadicBindParams): Promise<SQLiteRunResult>;
  public async runAsync(source: string, ...params: unknown[]): Promise<SQLiteRunResult> {
    await nextTurn();
    const { statement, named, positional } = this.prepare(source, params);
    const result = statement.run(named, ...positional);
    return { changes: Number(result.changes), lastInsertRowId: Number(result.lastInsertRowid) };
  }

  public getFirstAsync<T>(source: string, params: SQLiteBindParams): Promise<T | null>;
  public getFirstAsync<T>(source: string, ...params: SQLiteVariadicBindParams): Promise<T | null>;
  public async getFirstAsync<T>(source: string, ...params: unknown[]): Promise<T | null> {
    await nextTurn();
    const { statement, named, positional } = this.prepare(source, params);
    const row = statement.get(named, ...positional);
    return row === undefined ? null : ({ ...row } as T);
  }

  public getAllAsync<T>(source: string, params: SQLiteBindParams): Promise<T[]>;
  public getAllAsync<T>(source: string, ...params: SQLiteVariadicBindParams): Promise<T[]>;
  public async getAllAsync<T>(source: string, ...params: unknown[]): Promise<T[]> {
    await nextTurn();
    const { statement, named, positional } = this.prepare(source, params);
    return statement.all(named, ...positional).map((row) => ({ ...row }) as T);
  }

  private prepare(source: string, params: readonly unknown[]): PreparedCall {
    const statement = this.connection.prepare(source);
    const normalized = normalizeParams(params);
    if (isPositional(normalized)) {
      return { statement, named: {}, positional: normalized.map(toSqlValue) };
    }
    statement.setAllowBareNamedParameters(false);
    return { statement, named: namedValues(normalized), positional: [] };
  }
}

/**
 * A file-backed `node:sqlite` database with the statement surface of an Expo `SQLiteDatabase`
 * that `ExpoSqliteStore` and the root stores call. Exclusive transactions open a second
 * connection to the same file, as Expo does, so a root write that overlaps one fails with
 * `database is locked` instead of silently joining it.
 */
export class NodeSqliteDatabase extends NodeSqliteConnection implements ExpoSqliteDatabase {
  public readonly databasePath: string;
  private readonly ownedDirectory: string | undefined;

  /** Opens `databasePath`, or a new temporary file that `closeAsync()` deletes. */
  public constructor(databasePath?: string) {
    const file = resolveDatabaseFile(databasePath);
    super(new DatabaseSync(file.path, EXPO_CONNECTION_OPTIONS));
    this.databasePath = file.path;
    this.ownedDirectory = file.ownedDirectory;
  }

  /** Runs `BEGIN`, the task, and `COMMIT` on a new connection, rolling back if the task throws. */
  public async withExclusiveTransactionAsync(
    task: (transaction: NodeSqliteConnection) => Promise<void>,
  ): Promise<void> {
    await nextTurn();
    const connection = new DatabaseSync(this.databasePath, EXPO_CONNECTION_OPTIONS);
    try {
      connection.exec('BEGIN');
      await runInTransaction(connection, task);
    } finally {
      connection.close();
    }
  }

  public async closeAsync(): Promise<void> {
    await nextTurn();
    this.connection.close();
    if (this.ownedDirectory !== undefined) {
      rmSync(this.ownedDirectory, { recursive: true, force: true });
    }
  }
}

function resolveDatabaseFile(databasePath: string | undefined): DatabaseFile {
  if (databasePath === MEMORY_DATABASE_PATH || databasePath === '') {
    throw new TypeError(
      'NodeSqliteDatabase needs a file path: an exclusive transaction opens a second connection that must see the same database.',
    );
  }
  if (databasePath !== undefined) {
    return { path: databasePath, ownedDirectory: undefined };
  }
  const ownedDirectory = mkdtempSync(join(tmpdir(), TEMPORARY_DIRECTORY_PREFIX));
  return { path: join(ownedDirectory, TEMPORARY_DATABASE_FILE), ownedDirectory };
}

async function runInTransaction(
  connection: DatabaseSync,
  task: (transaction: NodeSqliteConnection) => Promise<void>,
): Promise<void> {
  try {
    await task(new NodeSqliteConnection(connection));
    connection.exec('COMMIT');
  } catch (cause) {
    if (connection.isTransaction) {
      connection.exec('ROLLBACK');
    }
    throw cause;
  }
}

/** Defers the statement to a later microtask, as a call into Expo's native module does. */
function nextTurn(): Promise<void> {
  return Promise.resolve();
}

/** Mirrors `expo-sqlite`'s parameter normalization: variadic values, one array, or one object. */
function normalizeParams(params: readonly unknown[]): readonly unknown[] | Record<string, unknown> {
  const candidate = params.length > 1 ? params : params[0];
  if (candidate === undefined || candidate === null) {
    return [];
  }
  if (
    typeof candidate !== 'object' ||
    candidate instanceof ArrayBuffer ||
    ArrayBuffer.isView(candidate)
  ) {
    return [candidate];
  }
  return candidate as readonly unknown[] | Record<string, unknown>;
}

function isPositional(
  params: readonly unknown[] | Record<string, unknown>,
): params is readonly unknown[] {
  return Array.isArray(params);
}

function namedValues(params: Record<string, unknown>): NamedValues {
  const values: NamedValues = {};
  for (const [key, value] of Object.entries(params)) {
    values[key] = toSqlValue(value);
  }
  return values;
}

/**
 * Binds values the way Expo's native module does: booleans and integral numbers as 64-bit
 * integers, `undefined` as NULL, and an `ArrayBuffer` as a blob.
 */
function toSqlValue(value: unknown): SQLInputValue {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value === 'boolean') {
    return value ? 1n : 0n;
  }
  if (typeof value === 'number' && Number.isSafeInteger(value)) {
    return BigInt(value);
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  return value as SQLInputValue;
}
