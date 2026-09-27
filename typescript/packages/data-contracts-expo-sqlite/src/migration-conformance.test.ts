import { describe, expect, it } from 'vitest';

import {
  createSqliteMigrationConformanceTests,
  type NodeSqliteConnection,
  type NodeSqliteDatabase,
  type SqliteMigrationConformanceAdapter,
  type SqliteMigrationStep,
} from './testing.js';

interface RunnerOptions {
  readonly transactional: boolean;
  readonly refusesNewer: boolean;
}

interface VersionRow {
  readonly version: number | null;
}

const HISTORY_TABLE = `CREATE TABLE IF NOT EXISTS migration_history (
  version INTEGER PRIMARY KEY NOT NULL,
  name TEXT NOT NULL
)`;

async function applyStep(
  connection: NodeSqliteConnection,
  step: SqliteMigrationStep,
): Promise<void> {
  await connection.execAsync(step.sql);
  await connection.runAsync(
    'INSERT INTO migration_history (version, name) VALUES (?, ?)',
    step.version,
    step.name,
  );
}

async function assertNotNewer(
  database: NodeSqliteDatabase,
  steps: readonly SqliteMigrationStep[],
): Promise<void> {
  const row = await database.getFirstAsync<VersionRow>(
    'SELECT max(version) AS version FROM migration_history',
  );
  const known = Math.max(0, ...steps.map((step) => step.version));
  if ((row?.version ?? 0) > known) {
    throw new Error(`database version ${String(row?.version)} is newer than ${String(known)}`);
  }
}

function historyRunner(options: RunnerOptions): SqliteMigrationConformanceAdapter {
  return {
    migrate: async (database, steps) => {
      await database.execAsync(HISTORY_TABLE);
      if (options.refusesNewer) {
        await assertNotNewer(database, steps);
      }
      const rows = await database.getAllAsync<VersionRow>('SELECT version FROM migration_history');
      const applied = new Set(rows.map((row) => row.version));
      const pending = [...steps]
        .sort((left, right) => left.version - right.version)
        .filter((step) => !applied.has(step.version));
      for (const step of pending) {
        if (options.transactional) {
          await database.withExclusiveTransactionAsync((transaction) =>
            applyStep(transaction, step),
          );
        } else {
          await applyStep(database, step);
        }
      }
    },
  };
}

const ADD_COLUMN = /^ALTER TABLE (\w+) ADD COLUMN (\w+)/i;

async function hasColumn(
  database: NodeSqliteDatabase,
  table: string,
  column: string,
): Promise<boolean> {
  const row = await database.getFirstAsync(
    'SELECT 1 FROM pragma_table_info(?) WHERE name = ?',
    table,
    column,
  );
  return row !== null;
}

async function runIdempotent(database: NodeSqliteDatabase, statement: string): Promise<void> {
  const addColumn = ADD_COLUMN.exec(statement);
  if (addColumn?.[1] !== undefined && addColumn[2] !== undefined) {
    if (!(await hasColumn(database, addColumn[1], addColumn[2]))) {
      await database.execAsync(statement);
    }
    return;
  }
  await database.execAsync(
    statement
      .replace(/^CREATE TABLE /i, 'CREATE TABLE IF NOT EXISTS ')
      .replace(/^INSERT INTO /i, 'INSERT OR IGNORE INTO '),
  );
}

/** Idempotent DDL and column probes with no history: re-runnable, but neither atomic nor version-aware. */
const versionlessRunner: SqliteMigrationConformanceAdapter = {
  migrate: async (database, steps) => {
    for (const step of steps) {
      const statements = step.sql
        .split(';')
        .map((statement) => statement.trim())
        .filter(Boolean);
      for (const statement of statements) {
        await runIdempotent(database, statement);
      }
    }
  },
};

async function failedCases(adapter: SqliteMigrationConformanceAdapter): Promise<string[]> {
  const failed: string[] = [];
  for (const testCase of createSqliteMigrationConformanceTests(adapter)) {
    try {
      await testCase.run();
    } catch {
      failed.push(testCase.name);
    }
  }
  return failed;
}

const ROLLBACK_CASE = 'rolls back a failed step completely and applies it after a fix';
const NEWER_BUILD_CASE = 'refuses a database written by a newer build and leaves it unchanged';

describe('createSqliteMigrationConformanceTests', () => {
  describe('with a transactional runner that refuses newer databases', () => {
    for (const testCase of createSqliteMigrationConformanceTests(
      historyRunner({ transactional: true, refusesNewer: true }),
    )) {
      it(testCase.name, testCase.run);
    }
  });

  it('catches a partially applied step when the runner skips the transaction', async () => {
    await expect(
      failedCases(historyRunner({ transactional: false, refusesNewer: true })),
    ).resolves.toEqual([ROLLBACK_CASE]);
  });

  it('catches a runner that accepts a database from a newer build', async () => {
    await expect(
      failedCases(historyRunner({ transactional: true, refusesNewer: false })),
    ).resolves.toEqual([NEWER_BUILD_CASE]);
  });

  it('catches both defects in a runner without version history', async () => {
    await expect(failedCases(versionlessRunner)).resolves.toEqual([
      ROLLBACK_CASE,
      NEWER_BUILD_CASE,
    ]);
  });

  it('names the failed expectation', async () => {
    const [newerBuild] = createSqliteMigrationConformanceTests(
      historyRunner({ transactional: true, refusesNewer: false }),
    ).filter((testCase) => testCase.name === NEWER_BUILD_CASE);
    await expect(newerBuild?.run()).rejects.toThrow(
      'SQLite migration conformance failed: an older build accepted a database that a newer build migrated',
    );
  });
});
