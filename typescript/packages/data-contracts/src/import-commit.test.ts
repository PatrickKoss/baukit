import { describe, expect, it, vi } from 'vitest';

import {
  commitImportEnvelope,
  ImportEnvelopeError,
  MAX_IMPORT_VALUE_DEPTH,
  prepareImportEnvelope,
  type ImportEnvelopePreview,
} from './import-envelope.js';

const transaction = {
  withTransaction: <T>(operation: (tx: undefined) => Promise<T>) => operation(undefined),
};

function prepare(value: unknown, subject?: string) {
  return prepareImportEnvelope({
    source: '{}',
    limits: { maxSourceBytes: 100, maxRows: 10, maxStringBytes: 100 },
    decodeEnvelope: () => ({ context: null, rows: [{ collection: 'items', fields: { value } }] }),
    fieldAllowlist: { items: ['value'] },
    decodeRow: ({ fields }) => fields,
    plan: ({ rows }) => rows,
    ...(subject === undefined ? {} : { subject }),
  });
}

function nested(depth: number): unknown {
  let value: unknown = 'leaf';
  for (let level = 0; level < depth; level += 1) value = [value];
  return value;
}

describe('commitImportEnvelope', () => {
  it('reports success when afterCommit throws and surfaces the hook error', async () => {
    const preview: ImportEnvelopePreview<string> = { rowCount: 0, plan: 'plan' };
    const hookError = new Error('cursor write failed');
    const onAfterCommitError = vi.fn();
    await expect(
      commitImportEnvelope({
        preview,
        transaction,
        write: () => Promise.resolve('committed'),
        afterCommit: () => {
          throw hookError;
        },
        onAfterCommitError,
      }),
    ).resolves.toBe('committed');
    expect(onAfterCommitError).toHaveBeenCalledWith(hookError, 'committed');
  });

  it('refuses a preview prepared for another subject', async () => {
    const preview = await prepare('x', 'account-a');
    const write = vi.fn();
    await expect(
      commitImportEnvelope({ preview, transaction, write, subject: 'account-b' }),
    ).rejects.toMatchObject({ code: 'subject_mismatch' });
    expect(write).not.toHaveBeenCalled();
  });
});

describe('import value depth', () => {
  it('accepts values up to the depth limit', async () => {
    await expect(prepare(nested(MAX_IMPORT_VALUE_DEPTH))).resolves.toMatchObject({ rowCount: 1 });
  });

  it('rejects deeper values with a typed error instead of a RangeError', async () => {
    const rejection = prepare(nested(100_000));
    await expect(rejection).rejects.toBeInstanceOf(ImportEnvelopeError);
    await expect(rejection).rejects.toMatchObject({ code: 'value_too_deep' });
  });
});
