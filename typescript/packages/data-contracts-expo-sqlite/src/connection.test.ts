import { StorageError, type StoredRecord } from '@baukit/data-contracts';
import { afterEach, describe, expect, it } from 'vitest';

import { ExpoSqliteConnection, ExpoSqliteStore, type SqliteTransaction } from './index.js';
import {
  NodeSqliteDatabase,
  type SqliteMigrationConformanceAdapter,
  type SqliteMigrationStep,
  createSqliteMigrationConformanceTests,
} from './testing.js';

interface Signal {
  readonly promise: Promise<void>;
  readonly resolve: () => void;
}

interface LabelRow {
  readonly label: string;
}

interface VersionRow {
  readonly version: number | null;
}

interface CachedRecord extends StoredRecord {
  readonly value: string;
}

/** The statements a root-transaction driver exposes, whether or not it queues them. */
interface RawDriver {
  exec(source: string): Promise<unknown>;
  run(source: string, ...params: string[]): Promise<unknown>;
  transaction(work: (transaction: RawDriver) => Promise<void>): Promise<void>;
}

const CREATE_EVENTS = 'CREATE TABLE events (label TEXT NOT NULL)';
const INSERT_EVENT = 'INSERT INTO events (label) VALUES (?)';
const CREATE_FAMILY = `CREATE TABLE parents (id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE children (
  id TEXT PRIMARY KEY NOT NULL,
  parent_id TEXT NOT NULL REFERENCES parents (id) ON DELETE CASCADE
);`;

const databases: NodeSqliteDatabase[] = [];

function openDatabase(): NodeSqliteDatabase {
  const database = new NodeSqliteDatabase();
  databases.push(database);
  return database;
}

function signal(): Signal {
  let resolve = (): void => undefined;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function flush(): Promise<void> {
  return new Promise((resolve) => {
    setImmediate(resolve);
  });
}

async function labels(database: NodeSqliteDatabase): Promise<string[]> {
  const rows = await database.getAllAsync<LabelRow>('SELECT label FROM events ORDER BY rowid');
  return rows.map((row) => row.label);
}

/** Expo's `withTransactionAsync` shape: `BEGIN` on the root handle with no queue. */
function unqueuedRootTransactions(database: NodeSqliteDatabase): RawDriver {
  const driver: RawDriver = {
    exec: (source) => database.execAsync(source),
    run: (source, ...params) => database.runAsync(source, ...params),
    transaction: async (work) => {
      await database.execAsync('BEGIN');
      try {
        await work(driver);
        await database.execAsync('COMMIT');
      } catch (cause) {
        await database.execAsync('ROLLBACK');
        throw cause;
      }
    },
  };
  return driver;
}

function queuedConnection(database: NodeSqliteDatabase): RawDriver {
  const connection = new ExpoSqliteConnection(database);
  return {
    exec: (source) => connection.exec(source),
    run: (source, ...params) => connection.run(source, ...params),
    transaction: (work) =>
      connection.transaction((transaction) =>
        work({
          exec: (source) => transaction.exec(source),
          run: (source, ...params) => transaction.run(source, ...params),
          transaction: (nested) => transaction.transaction(nested),
        }),
      ),
  };
}

/**
 * One caller's transaction writes a row and rolls back while another caller issues an
 * unrelated root write. Returns the rows that survive.
 */
async function rootWriteDuringRolledBackTransaction(
  database: NodeSqliteDatabase,
  driver: RawDriver,
): Promise<string[]> {
  await driver.exec(CREATE_EVENTS);
  const entered = signal();
  const release = signal();
  const transaction = driver.transaction(async (context) => {
    await context.run(INSERT_EVENT, 'transaction');
    entered.resolve();
    await release.promise;
    throw new Error('roll back');
  });
  await entered.promise;
  const root = driver.run(INSERT_EVENT, 'root');
  await flush();
  release.resolve();
  await expect(transaction).rejects.toThrow('roll back');
  await root;
  return labels(database);
}

afterEach(async () => {
  await Promise.allSettled(databases.splice(0).map((database) => database.closeAsync()));
});

describe('ExpoSqliteConnection', () => {
  it('runs, reads, and binds parameters as Expo does', async () => {
    const connection = new ExpoSqliteConnection(openDatabase());
    await connection.exec('CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT, flag ANY)');
    const inserted = await connection.run(
      'INSERT INTO items (label, flag) VALUES (?, ?)',
      'variadic',
      true,
    );
    await connection.run('INSERT INTO items (label, flag) VALUES (?, ?)', ['array', null]);
    await connection.run('INSERT INTO items (label, flag) VALUES ($label, $flag)', {
      $label: 'named',
      $flag: false,
    });
    expect(inserted).toEqual({ changes: 1, lastInsertRowId: 1 });
    await expect(
      connection.all<{ readonly label: string; readonly flag: number | null }>(
        'SELECT label, flag FROM items ORDER BY id',
      ),
    ).resolves.toEqual([
      { label: 'variadic', flag: 1 },
      { label: 'array', flag: null },
      { label: 'named', flag: 0 },
    ]);
    await expect(
      connection.get<LabelRow>('SELECT label FROM items WHERE id = ?', 3),
    ).resolves.toEqual({ label: 'named' });
    await expect(connection.get('SELECT label FROM items WHERE id = ?', 99)).resolves.toBe(
      undefined,
    );
  });

  it('keeps a root statement issued during a transaction out of that transaction', async () => {
    const database = openDatabase();
    await expect(
      rootWriteDuringRolledBackTransaction(database, queuedConnection(database)),
    ).resolves.toEqual(['root']);
  });

  it('shows the same case losing the root write when root transactions are not queued', async () => {
    const database = openDatabase();
    await expect(
      rootWriteDuringRolledBackTransaction(database, unqueuedRootTransactions(database)),
    ).resolves.toEqual([]);
  });

  it('commits every statement and returns the callback result', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(CREATE_EVENTS);
    const result = await connection.transaction(async (transaction) => {
      await transaction.run(INSERT_EVENT, 'first');
      await transaction.run(INSERT_EVENT, 'second');
      return transaction.all<LabelRow>('SELECT label FROM events ORDER BY rowid');
    });
    expect(result).toEqual([{ label: 'first' }, { label: 'second' }]);
    await expect(labels(database)).resolves.toEqual(['first', 'second']);
  });

  it('rolls back every statement, including schema changes, and rethrows the failure', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(CREATE_EVENTS);
    const failure = new Error('step failed');
    await expect(
      connection.transaction(async (transaction) => {
        await transaction.run(INSERT_EVENT, 'lost');
        await transaction.exec('CREATE TABLE partial (id TEXT); ALTER TABLE events ADD COLUMN x');
        throw failure;
      }),
    ).rejects.toBe(failure);
    await expect(labels(database)).resolves.toEqual([]);
    await expect(
      connection.get("SELECT name FROM sqlite_master WHERE name = 'partial'"),
    ).resolves.toBe(undefined);
    await expect(connection.all("SELECT name FROM pragma_table_info('events')")).resolves.toEqual([
      { name: 'label' },
    ]);
  });

  it('rolls back when the commit fails and passes the commit error on', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(`CREATE TABLE parents (id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE children (
  id TEXT PRIMARY KEY NOT NULL,
  parent_id TEXT NOT NULL REFERENCES parents (id) DEFERRABLE INITIALLY DEFERRED
);`);
    await expect(
      connection.transaction((transaction) =>
        transaction.run('INSERT INTO children (id, parent_id) VALUES (?, ?)', 'orphan', 'none'),
      ),
    ).rejects.toThrow('FOREIGN KEY constraint failed');
    await expect(connection.all('SELECT id FROM children')).resolves.toEqual([]);
    await expect(connection.run('INSERT INTO parents (id) VALUES (?)', 'next')).resolves.toEqual({
      changes: 1,
      lastInsertRowId: 1,
    });
  });

  it('enforces foreign keys inside transactions', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(CREATE_FAMILY);
    await expect(
      connection.transaction((transaction) =>
        transaction.run('INSERT INTO children (id, parent_id) VALUES (?, ?)', 'orphan', 'none'),
      ),
    ).rejects.toThrow('FOREIGN KEY constraint failed');
    await connection.transaction(async (transaction) => {
      await transaction.run('INSERT INTO parents (id) VALUES (?)', 'parent');
      await transaction.run('INSERT INTO children (id, parent_id) VALUES (?, ?)', 'kid', 'parent');
      await transaction.run('DELETE FROM parents WHERE id = ?', 'parent');
    });
    await expect(database.getAllAsync('SELECT id FROM children')).resolves.toEqual([]);
  });

  it('enables foreign keys before its first statement on the handle', async () => {
    const database = openDatabase();
    await expect(database.getFirstAsync('PRAGMA foreign_keys')).resolves.toEqual({
      foreign_keys: 0,
    });
    const connection = new ExpoSqliteConnection(database);
    await expect(connection.get('PRAGMA foreign_keys')).resolves.toEqual({ foreign_keys: 1 });
  });

  it('rejects a nested transaction instead of joining it', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(CREATE_EVENTS);
    await connection.transaction(async (transaction) => {
      await transaction.run(INSERT_EVENT, 'outer');
      await expect(
        transaction.transaction((nested) => nested.run(INSERT_EVENT, 'nested')),
      ).rejects.toThrow(
        'A SQLite transaction cannot start another transaction. Run the statements on the current transaction instead.',
      );
    });
    await expect(labels(database)).resolves.toEqual(['outer']);
  });

  it('rejects statements on a transaction after it settles', async () => {
    const connection = new ExpoSqliteConnection(openDatabase());
    await connection.exec(CREATE_EVENTS);
    let leaked: SqliteTransaction | undefined;
    await connection.transaction((transaction) => {
      leaked = transaction;
    });
    const error = await leaked?.run(INSERT_EVENT, 'late').catch((cause: unknown) => cause);
    expect(error).toBeInstanceOf(StorageError);
    expect(error).toMatchObject({ code: 'storage_closed' });
    await expect(connection.all('SELECT label FROM events')).resolves.toEqual([]);
  });

  it('runs root statements and transactions one at a time in call order', async () => {
    const connection = new ExpoSqliteConnection(openDatabase());
    await connection.exec(CREATE_EVENTS);
    const events: string[] = [];
    const first = connection.transaction(async (transaction) => {
      events.push('first:start');
      await transaction.run(INSERT_EVENT, 'first');
      await flush();
      events.push('first:end');
    });
    const root = connection.run(INSERT_EVENT, 'root').then(() => {
      events.push('root');
    });
    const second = connection.transaction(async (transaction) => {
      events.push('second:start');
      await transaction.run(INSERT_EVENT, 'second');
      events.push('second:end');
    });
    await Promise.all([first, root, second]);
    expect(events).toEqual(['first:start', 'first:end', 'root', 'second:start', 'second:end']);
  });

  it('releases the queue after a failed statement', async () => {
    const connection = new ExpoSqliteConnection(openDatabase());
    const failed = connection.exec('SELECT * FROM missing_table');
    const next = connection.get<{ readonly one: number }>('SELECT 1 AS one');
    await expect(failed).rejects.toThrow('no such table');
    await expect(next).resolves.toEqual({ one: 1 });
  });

  it('shares the file queue with ExpoSqliteStore in both arrival orders', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    const store = new ExpoSqliteStore<CachedRecord>(database, 'cache');
    await store.initialize();
    await connection.exec(CREATE_EVENTS);

    const storeEntered = signal();
    const storeRelease = signal();
    const storeTransaction = store.withTransaction(async (context) => {
      await context.records.put({ id: 'store', value: 'committed' });
      storeEntered.resolve();
      await storeRelease.promise;
    });
    await storeEntered.promise;
    const rootWrite = connection.run(INSERT_EVENT, 'after store');
    await flush();
    storeRelease.resolve();
    await Promise.all([storeTransaction, rootWrite]);

    const connectionEntered = signal();
    const connectionRelease = signal();
    const connectionTransaction = connection.transaction(async (transaction) => {
      await transaction.run(INSERT_EVENT, 'rolled back');
      connectionEntered.resolve();
      await connectionRelease.promise;
      throw new Error('roll back');
    });
    await connectionEntered.promise;
    const storeWrite = store.records.put({ id: 'root', value: 'kept' });
    await flush();
    connectionRelease.resolve();
    await expect(connectionTransaction).rejects.toThrow('roll back');
    await storeWrite;

    await expect(labels(database)).resolves.toEqual(['after store']);
    await expect(store.records.get('root')).resolves.toEqual({ id: 'root', value: 'kept' });
  });

  it('closes an owned handle once after accepted work and rejects later calls', async () => {
    const database = new NodeSqliteDatabase();
    const connection = new ExpoSqliteConnection(database, { closeDatabase: true });
    await connection.exec(CREATE_EVENTS);
    const accepted = connection.transaction((transaction) => transaction.run(INSERT_EVENT, 'x'));
    const closed = connection.close();
    expect(connection.close()).toBe(closed);
    await expect(accepted).resolves.toMatchObject({ changes: 1 });
    await closed;
    await expect(connection.run(INSERT_EVENT, 'late')).rejects.toMatchObject({
      code: 'storage_closed',
    });
    await expect(connection.transaction(() => undefined)).rejects.toMatchObject({
      code: 'storage_closed',
    });
    await expect(database.execAsync('SELECT 1')).rejects.toThrow();
  });

  it('leaves an unowned handle open on close', async () => {
    const database = openDatabase();
    const connection = new ExpoSqliteConnection(database);
    await connection.close();
    await expect(database.getFirstAsync('SELECT 1 AS one')).resolves.toEqual({ one: 1 });
  });
});

const HISTORY_TABLE = `CREATE TABLE IF NOT EXISTS migration_history (
  version INTEGER PRIMARY KEY NOT NULL,
  name TEXT NOT NULL
)`;

async function assertNotNewer(
  connection: ExpoSqliteConnection,
  steps: readonly SqliteMigrationStep[],
): Promise<void> {
  const row = await connection.get<VersionRow>(
    'SELECT max(version) AS version FROM migration_history',
  );
  const known = Math.max(0, ...steps.map((step) => step.version));
  if ((row?.version ?? 0) > known) {
    throw new Error(`database version ${String(row?.version)} is newer than ${String(known)}`);
  }
}

const connectionRunner: SqliteMigrationConformanceAdapter = {
  migrate: async (database, steps) => {
    const connection = new ExpoSqliteConnection(database);
    await connection.exec(HISTORY_TABLE);
    await assertNotNewer(connection, steps);
    const rows = await connection.all<VersionRow>('SELECT version FROM migration_history');
    const applied = new Set(rows.map((row) => row.version));
    const pending = [...steps]
      .sort((left, right) => left.version - right.version)
      .filter((step) => !applied.has(step.version));
    for (const step of pending) {
      await connection.transaction(async (transaction) => {
        await transaction.exec(step.sql);
        await transaction.run(
          'INSERT INTO migration_history (version, name) VALUES (?, ?)',
          step.version,
          step.name,
        );
      });
    }
  },
};

describe('createSqliteMigrationConformanceTests with a history runner on ExpoSqliteConnection', () => {
  for (const testCase of createSqliteMigrationConformanceTests(connectionRunner)) {
    it(testCase.name, testCase.run);
  }
});
