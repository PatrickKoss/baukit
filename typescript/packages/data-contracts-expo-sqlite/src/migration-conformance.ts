import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { NodeSqliteDatabase } from './node-sqlite.js';

/** One versioned schema change, in the shape most product runners already accept. */
export interface SqliteMigrationStep {
  readonly version: number;
  readonly name: string;
  readonly sql: string;
}

export interface SqliteMigrationConformanceAdapter {
  /**
   * Runs the product's migration runner over `steps`, the complete list one app build ships,
   * usually by wrapping `database` in the product's own Expo driver. Rejects when the runner
   * fails or refuses the database.
   */
  migrate(database: NodeSqliteDatabase, steps: readonly SqliteMigrationStep[]): Promise<void>;
}

export interface SqliteMigrationConformanceTestCase {
  readonly name: string;
  readonly run: () => Promise<void>;
}

interface NameRow {
  readonly name: string;
}

interface CountRow {
  readonly count: number;
}

const ITEMS_TABLE = 'conformance_items';
const SETTINGS_TABLE = 'conformance_settings';
const PARTIAL_TABLE = 'conformance_partial';
const RANK_COLUMN = 'rank';

const CREATE_ITEMS: SqliteMigrationStep = {
  version: 1,
  name: 'conformance_create_items',
  sql: `CREATE TABLE ${ITEMS_TABLE} (id TEXT PRIMARY KEY NOT NULL, label TEXT NOT NULL);`,
};

const SEED_SETTINGS: SqliteMigrationStep = {
  version: 2,
  name: 'conformance_seed_settings',
  sql: `CREATE TABLE ${SETTINGS_TABLE} (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
INSERT INTO ${SETTINGS_TABLE} (key, value) VALUES ('seeded', 'once');`,
};

const ADD_RANK: SqliteMigrationStep = {
  version: 3,
  name: 'conformance_add_item_rank',
  sql: `ALTER TABLE ${ITEMS_TABLE} ADD COLUMN ${RANK_COLUMN} INTEGER NOT NULL DEFAULT 0;`,
};

const FAILING_ADD_RANK: SqliteMigrationStep = {
  ...ADD_RANK,
  sql: `${ADD_RANK.sql}
CREATE TABLE ${PARTIAL_TABLE} (id TEXT);
INSERT INTO conformance_missing_table (id) VALUES ('fails');`,
};

const NEWER_BUILD_STEP: SqliteMigrationStep = {
  version: 4,
  name: 'conformance_newer_build',
  sql: 'CREATE TABLE conformance_newer_build (id TEXT);',
};

const CURRENT_STEPS = [CREATE_ITEMS, SEED_SETTINGS, ADD_RANK] as const;

class ConformanceFailure extends Error {
  public constructor(detail: string) {
    super(`SQLite migration conformance failed: ${detail}`);
    this.name = 'ConformanceFailure';
  }
}

function assert(condition: boolean, detail: string): asserts condition {
  if (!condition) {
    throw new ConformanceFailure(detail);
  }
}

/** One database file that a case can close and reopen, removed when the case ends. */
class ConformanceFile {
  private readonly open = new Set<NodeSqliteDatabase>();

  public constructor(private readonly path: string) {}

  public connect(): NodeSqliteDatabase {
    const database = new NodeSqliteDatabase(this.path);
    this.open.add(database);
    return database;
  }

  public async restart(database: NodeSqliteDatabase): Promise<NodeSqliteDatabase> {
    this.open.delete(database);
    await database.closeAsync();
    return this.connect();
  }

  public async closeAll(): Promise<void> {
    const handles = [...this.open];
    this.open.clear();
    await Promise.allSettled(handles.map((database) => database.closeAsync()));
  }
}

async function withDatabaseFile(run: (file: ConformanceFile) => Promise<void>): Promise<void> {
  const directory = await mkdtemp(join(tmpdir(), 'baukit-sqlite-migrations-'));
  const file = new ConformanceFile(join(directory, 'database.sqlite'));
  try {
    await run(file);
  } finally {
    await file.closeAll();
    await rm(directory, { recursive: true, force: true });
  }
}

async function rejects(operation: () => Promise<void>): Promise<boolean> {
  try {
    await operation();
    return false;
  } catch {
    return true;
  }
}

function quoteIdentifier(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

async function tableNames(database: NodeSqliteDatabase): Promise<Set<string>> {
  const rows = await database.getAllAsync<NameRow>(
    "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
  );
  return new Set(rows.map((row) => row.name));
}

async function columnNames(database: NodeSqliteDatabase, table: string): Promise<Set<string>> {
  const rows = await database.getAllAsync<NameRow>(
    `SELECT name FROM pragma_table_info(${quoteSqlText(table)})`,
  );
  return new Set(rows.map((row) => row.name));
}

async function rowCount(database: NodeSqliteDatabase, table: string): Promise<number> {
  const row = await database.getFirstAsync<CountRow>(
    `SELECT count(*) AS count FROM ${quoteIdentifier(table)}`,
  );
  return row?.count ?? 0;
}

function quoteSqlText(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

/** Schema objects, `user_version`, and every table's rows, for an unchanged-database check. */
async function snapshot(database: NodeSqliteDatabase): Promise<string> {
  const objects = await database.getAllAsync<{ readonly type: string; readonly name: string }>(
    'SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name',
  );
  const userVersion = await database.getFirstAsync('PRAGMA user_version');
  const tables: Record<string, unknown[]> = {};
  for (const object of objects.filter((candidate) => candidate.type === 'table')) {
    tables[object.name] = await database.getAllAsync(
      `SELECT * FROM ${quoteIdentifier(object.name)} ORDER BY rowid`,
    );
  }
  return JSON.stringify({ objects, userVersion, tables });
}

async function assertCurrentSchema(database: NodeSqliteDatabase): Promise<void> {
  const tables = await tableNames(database);
  assert(tables.has(ITEMS_TABLE), `${ITEMS_TABLE} is missing after migration`);
  assert(tables.has(SETTINGS_TABLE), `${SETTINGS_TABLE} is missing after migration`);
  const columns = await columnNames(database, ITEMS_TABLE);
  assert(columns.has(RANK_COLUMN), `${ITEMS_TABLE}.${RANK_COLUMN} is missing after migration`);
  const seeded = await rowCount(database, SETTINGS_TABLE);
  assert(seeded === 1, `step 2 ran ${String(seeded)} times instead of once`);
}

/**
 * Builds framework-neutral cases for a product's SQLite migration runner: fresh install,
 * upgrade, restart, rollback of a failed step, and refusal of a database written by a newer
 * build. Register each case with the test runner's `it`.
 */
export function createSqliteMigrationConformanceTests(
  adapter: SqliteMigrationConformanceAdapter,
): readonly SqliteMigrationConformanceTestCase[] {
  return [
    {
      name: 'applies every step to a fresh database',
      run: () =>
        withDatabaseFile(async (file) => {
          const database = file.connect();
          await adapter.migrate(database, CURRENT_STEPS);
          await assertCurrentSchema(database);
        }),
    },
    {
      name: 'upgrades an older database and keeps its rows',
      run: () =>
        withDatabaseFile(async (file) => {
          let database = file.connect();
          await adapter.migrate(database, [CREATE_ITEMS]);
          await database.runAsync(
            `INSERT INTO ${ITEMS_TABLE} (id, label) VALUES (?, ?)`,
            'kept',
            'written before the upgrade',
          );
          database = await file.restart(database);
          await adapter.migrate(database, CURRENT_STEPS);
          await assertCurrentSchema(database);
          const kept = await database.getFirstAsync<{ readonly rank: number }>(
            `SELECT ${RANK_COLUMN} AS rank FROM ${ITEMS_TABLE} WHERE id = ?`,
            'kept',
          );
          assert(kept?.rank === 0, 'a row written before the upgrade was lost or not defaulted');
        }),
    },
    {
      name: 'applies nothing again after a restart',
      run: () =>
        withDatabaseFile(async (file) => {
          let database = file.connect();
          await adapter.migrate(database, CURRENT_STEPS);
          const before = await snapshot(database);
          database = await file.restart(database);
          await adapter.migrate(database, CURRENT_STEPS);
          await assertCurrentSchema(database);
          assert(
            (await snapshot(database)) === before,
            'running the same steps again changed the database',
          );
        }),
    },
    {
      name: 'rolls back a failed step completely and applies it after a fix',
      run: () =>
        withDatabaseFile(async (file) => {
          let database = file.connect();
          const failed = await rejects(() =>
            adapter.migrate(database, [CREATE_ITEMS, SEED_SETTINGS, FAILING_ADD_RANK]),
          );
          assert(failed, 'a step whose last statement fails did not reject');
          assert(
            !(await tableNames(database)).has(PARTIAL_TABLE),
            'a failed step left a table from its earlier statements (partially applied migration)',
          );
          assert(
            !(await columnNames(database, ITEMS_TABLE)).has(RANK_COLUMN),
            'a failed step left a column from its earlier statements (partially applied migration)',
          );
          database = await file.restart(database);
          await adapter.migrate(database, CURRENT_STEPS);
          await assertCurrentSchema(database);
        }),
    },
    {
      name: 'refuses a database written by a newer build and leaves it unchanged',
      run: () =>
        withDatabaseFile(async (file) => {
          let database = file.connect();
          await adapter.migrate(database, [...CURRENT_STEPS, NEWER_BUILD_STEP]);
          database = await file.restart(database);
          const before = await snapshot(database);
          const refused = await rejects(() => adapter.migrate(database, CURRENT_STEPS));
          assert(refused, 'an older build accepted a database that a newer build migrated');
          assert(
            (await snapshot(database)) === before,
            'refusing a newer database still changed it',
          );
        }),
    },
  ];
}
