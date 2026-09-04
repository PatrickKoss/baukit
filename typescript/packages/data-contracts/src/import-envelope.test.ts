import { expect, it, vi } from 'vitest';

import fixtureCorpus from '../../../../fixtures/import-envelope/import-envelope-v1.json';

import {
  ImportEnvelopeError,
  type ImportEnvelopeLimits,
  type ImportEnvelopePreview,
  type ImportEnvelopeSource,
  type ImportEnvelopeTransactionAdapter,
  commitImportEnvelope,
  prepareImportEnvelope,
} from './import-envelope.js';
import {
  type ImportEnvelopeConformanceFixtures,
  type ImportEnvelopeConformanceHarness,
  type ImportEnvelopeConformanceHarnessOptions,
  describeImportEnvelopeContract,
} from './import-envelope.vitest.js';

interface FixtureCase {
  readonly name: string;
  readonly outcome: 'accept' | 'reject' | 'commit-failure';
  readonly source: unknown;
}

interface FixtureCorpus {
  readonly fixture_version: number;
  readonly limits: {
    readonly max_source_bytes: number;
    readonly max_rows: number;
    readonly max_string_bytes: number;
  };
  readonly cases: readonly FixtureCase[];
}

interface FixtureRow {
  readonly id: string;
  readonly title: string;
}

interface FixturePlan {
  readonly rows: readonly FixtureRow[];
}

interface FixtureTransaction {
  readonly rows: Map<string, FixtureRow>;
}

const corpus = fixtureCorpus as FixtureCorpus;
const limits: ImportEnvelopeLimits = {
  maxSourceBytes: corpus.limits.max_source_bytes,
  maxRows: corpus.limits.max_rows,
  maxStringBytes: corpus.limits.max_string_bytes,
};

function fixture(name: string): string {
  const entry = corpus.cases.find((candidate) => candidate.name === name);
  if (entry === undefined) throw new Error(`Missing import-envelope fixture: ${name}`);
  return JSON.stringify(entry.source);
}

const conformanceFixtures: ImportEnvelopeConformanceFixtures = {
  valid: fixture('valid'),
  unknownFields: fixture('unknown-fields'),
  duplicateIds: fixture('duplicate-ids'),
  tombstones: fixture('tombstones'),
  ownershipAndRevisionFields: fixture('ownership-and-revision-fields'),
  oversizedStrings: fixture('oversized-strings'),
  excessRows: fixture('excess-rows'),
  mixedVersions: fixture('mixed-versions'),
  failureHalfwayThroughCommit: fixture('failure-halfway-through-commit'),
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function decodeFixtureEnvelope(source: ImportEnvelopeSource) {
  if (typeof source !== 'string') throw new Error('fixture_source_invalid');
  const value: unknown = JSON.parse(source);
  if (!isRecord(value) || value['format'] !== 'baukit-test-import') {
    throw new Error('fixture_envelope_invalid');
  }
  const versions = value['schema_versions'];
  if (!Array.isArray(versions) || versions.length !== 1 || versions[0] !== 1) {
    throw new Error('fixture_schema_unsupported');
  }
  const rawRows = value['rows'];
  if (!Array.isArray(rawRows)) throw new Error('fixture_rows_invalid');

  const ids = new Set<string>();
  const rows = rawRows.map((raw) => {
    if (!isRecord(raw) || raw['collection'] !== 'records' || typeof raw['id'] !== 'string') {
      throw new Error('fixture_row_invalid');
    }
    const key = `${raw['collection']}:${raw['id']}`;
    if (ids.has(key)) throw new Error('fixture_duplicate_id');
    ids.add(key);
    if (raw['deleted_at'] !== undefined && raw['deleted_at'] !== null) {
      throw new Error('fixture_tombstone_unsupported');
    }
    const fields = Object.fromEntries(
      Object.entries(raw).filter(([field]) => field !== 'collection'),
    );
    return { collection: 'records' as const, fields };
  });
  return { context: { schemaVersion: 1 }, rows };
}

function prepareFixtureImport(
  source: ImportEnvelopeSource,
): Promise<ImportEnvelopePreview<FixturePlan>> {
  return prepareImportEnvelope({
    source,
    limits,
    decodeEnvelope: decodeFixtureEnvelope,
    fieldAllowlist: { records: ['id', 'title'] },
    decodeRow: ({ fields }) => {
      const id = fields['id'];
      const title = fields['title'];
      if (typeof id !== 'string' || typeof title !== 'string') {
        throw new Error('fixture_row_invalid');
      }
      return { id, title };
    },
    plan: ({ rows }) => ({ rows }),
  });
}

function makeHarness(
  options: ImportEnvelopeConformanceHarnessOptions,
): ImportEnvelopeConformanceHarness<ImportEnvelopePreview<FixturePlan>, string, string> {
  let committedRows = new Map<string, FixtureRow>();
  const writeFields: string[][] = [];
  let cursor = 'cursor-before';
  let syncState = 'idle';
  let transactionCommitted = false;
  let stateAdvancedBeforeCommit = false;

  const transaction: ImportEnvelopeTransactionAdapter<FixtureTransaction> = {
    async withTransaction(operation) {
      const workingRows = new Map(committedRows);
      transactionCommitted = false;
      const result = await operation({ rows: workingRows });
      committedRows = workingRows;
      transactionCommitted = true;
      return result;
    },
  };

  return {
    preview: prepareFixtureImport,
    async commit(preview) {
      let writes = 0;
      await commitImportEnvelope({
        preview,
        transaction,
        write: (activeTransaction, plan) => {
          for (const row of plan.rows) {
            writeFields.push(Object.keys(row).sort());
            activeTransaction.rows.set(row.id, row);
            writes += 1;
            if (writes === options.failAfterWrites) throw new Error('fixture_commit_failure');
          }
          return Promise.resolve();
        },
        afterCommit: () => {
          if (!transactionCommitted) stateAdvancedBeforeCommit = true;
          cursor = 'cursor-after';
          syncState = 'sync-requested';
        },
      });
    },
    observe: () => ({
      committedIds: [...committedRows.keys()].sort(),
      writeFields,
      cursor,
      syncState,
      stateAdvancedBeforeCommit,
    }),
  };
}

describeImportEnvelopeContract({
  fixtures: conformanceFixtures,
  makeHarness,
  expectedCommittedIds: ['row-a', 'row-b'],
  forbiddenWriteFields: ['owner_id', 'revision'],
  initialCursor: 'cursor-before',
  committedCursor: 'cursor-after',
  initialSyncState: 'idle',
  committedSyncState: 'sync-requested',
});

it.each([
  ['unknown-fields', { code: 'field_not_allowed', field: 'debug' }],
  ['ownership-and-revision-fields', { code: 'field_not_allowed', field: 'owner_id' }],
  ['oversized-strings', { code: 'string_too_large', field: 'title', measured: 17, allowed: 16 }],
  ['excess-rows', { code: 'too_many_rows', measured: 3, allowed: 2 }],
] as const)('returns bounded details for %s', async (name, expected) => {
  await expect(prepareFixtureImport(fixture(name))).rejects.toMatchObject(expected);
});

it('checks the source byte limit before calling the envelope decoder', async () => {
  const decodeEnvelope = vi.fn(decodeFixtureEnvelope);
  await expect(
    prepareImportEnvelope({
      source: fixture('valid'),
      limits: { ...limits, maxSourceBytes: 1 },
      decodeEnvelope,
      fieldAllowlist: { records: ['id', 'title'] },
      decodeRow: () => ({ id: 'unused', title: 'unused' }),
      plan: ({ rows }) => ({ rows }),
    }),
  ).rejects.toMatchObject({ code: 'source_too_large', allowed: 1 });
  expect(decodeEnvelope).not.toHaveBeenCalled();
});

it('rejects collections that have no field allowlist', async () => {
  await expect(
    prepareImportEnvelope({
      source: 'x',
      limits,
      decodeEnvelope: () => ({
        context: undefined,
        rows: [{ collection: 'unlisted', fields: { id: 'row-a' } }],
      }),
      fieldAllowlist: {},
      decodeRow: ({ fields }) => fields,
      plan: ({ rows }) => rows,
    }),
  ).rejects.toEqual(
    new ImportEnvelopeError('collection_not_allowed', {
      collection: 'unlisted',
      rowIndex: 0,
    }),
  );
});

it('checks strings nested inside an allowed field', async () => {
  await expect(
    prepareImportEnvelope({
      source: 'x',
      limits: { ...limits, maxStringBytes: 4 },
      decodeEnvelope: () => ({
        context: undefined,
        rows: [{ collection: 'records', fields: { payload: { label: '12345' } } }],
      }),
      fieldAllowlist: { records: ['payload'] },
      decodeRow: ({ fields }) => fields,
      plan: ({ rows }) => rows,
    }),
  ).rejects.toMatchObject({
    code: 'string_too_large',
    collection: 'records',
    rowIndex: 0,
    field: 'payload',
    measured: 5,
    allowed: 4,
  });
});

it('rejects invalid limits before decoding', async () => {
  const decodeEnvelope = vi.fn(decodeFixtureEnvelope);
  await expect(
    prepareImportEnvelope({
      source: 'x',
      limits: { ...limits, maxRows: -1 },
      decodeEnvelope,
      fieldAllowlist: { records: ['id'] },
      decodeRow: ({ fields }) => fields,
      plan: ({ rows }) => rows,
    }),
  ).rejects.toThrow('maxRows must be a non-negative safe integer');
  expect(decodeEnvelope).not.toHaveBeenCalled();
});

it('exports the helpers without removing the existing package entry points', async () => {
  const [root, entry, vitestEntry] = await Promise.all([
    import('@baukit/data-contracts'),
    import('@baukit/data-contracts/import-envelope'),
    import('@baukit/data-contracts/vitest'),
  ]);
  expect(root.InMemoryStore).toBeTypeOf('function');
  expect(root.prepareImportEnvelope).toBe(entry.prepareImportEnvelope);
  expect(entry.commitImportEnvelope).toBeTypeOf('function');
  expect(vitestEntry.describeRecordStoreContract).toBeTypeOf('function');
  expect(vitestEntry.describeImportEnvelopeContract).toBeTypeOf('function');
});
