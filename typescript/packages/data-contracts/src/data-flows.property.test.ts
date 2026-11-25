import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { AmbiguousProductProfileErasureError, eraseProductProfile } from './erasure.js';
import { encodeCsv } from './export.js';
import {
  commitImportEnvelope,
  ImportEnvelopeError,
  prepareImportEnvelope,
} from './import-envelope.js';

const TRIGGERS = ['=', '+', '-', '@', '＝', '＋', '－', '＠', '\t', '\r'];

/** Minimal RFC 4180 parser for CRLF-terminated records. */
function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = '';
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const char = text.charAt(index);
    if (quoted) {
      if (char === '"' && text.charAt(index + 1) === '"') {
        cell += '"';
        index += 1;
      } else if (char === '"') quoted = false;
      else cell += char;
    } else if (char === '"') quoted = true;
    else if (char === ',') {
      row.push(cell);
      cell = '';
    } else if (char === '\r' && text.charAt(index + 1) === '\n') {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = '';
      index += 1;
    } else cell += char;
  }
  return rows;
}

const text = fc.string({ unit: 'grapheme', maxLength: 8 });
const textWithTriggers = fc.oneof(
  text,
  fc.tuple(fc.constantFrom(...TRIGGERS, ' ', '"', ','), text).map(([lead, rest]) => lead + rest),
);

describe('C-05 import, export and erasure', () => {
  it('encodeCsv round-trips and never leaves a trigger first', () => {
    fc.assert(
      fc.property(
        fc.array(fc.array(textWithTriggers, { minLength: 2, maxLength: 4 }), { maxLength: 5 }),
        (rows) => {
          const parsed = parseCsv(encodeCsv(rows));
          expect(parsed).toHaveLength(rows.length);
          parsed.forEach((record, rowIndex) => {
            record.forEach((cell, columnIndex) => {
              const original = rows[rowIndex]?.[columnIndex] ?? '';
              expect(cell === original || cell === `'${original}`).toBe(true);
              const trimmed = cell.replace(/^[ \t\r\n]*/u, '');
              expect(
                TRIGGERS.includes(cell.charAt(0)) || TRIGGERS.includes(trimmed.charAt(0)),
              ).toBe(false);
            });
          });
        },
      ),
    );
  });

  it('prepareImportEnvelope returns a bounded preview or ImportEnvelopeError', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.array(fc.dictionary(fc.constantFrom('title', 'body', 'owner_id'), fc.jsonValue()), {
          maxLength: 6,
        }),
        fc.nat(4),
        async (rows, maxRows) => {
          try {
            const preview = await prepareImportEnvelope({
              source: JSON.stringify(rows),
              limits: { maxSourceBytes: 10_000, maxRows, maxStringBytes: 16 },
              decodeEnvelope: () => ({
                context: null,
                rows: rows.map((fields) => ({ collection: 'items' as const, fields })),
              }),
              fieldAllowlist: { items: ['title', 'body'] },
              decodeRow: ({ fields }) => fields,
              plan: ({ rows: decoded }) => decoded,
            });
            expect(preview.rowCount).toBeLessThanOrEqual(maxRows);
          } catch (cause) {
            expect(cause).toBeInstanceOf(ImportEnvelopeError);
          }
        },
      ),
    );
  });

  it('a write failing at step k leaves committed state unchanged', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.array(fc.string(), { minLength: 1, maxLength: 6 }),
        fc.nat(6),
        async (ids, failAt) => {
          let committed = new Map<string, string>([['existing', 'kept']]);
          const before = new Map(committed);
          const transaction = {
            async withTransaction<T>(
              operation: (tx: Map<string, string>) => Promise<T>,
            ): Promise<T> {
              const working = new Map(committed);
              const result = await operation(working);
              committed = working;
              return result;
            },
          };
          const outcome = commitImportEnvelope({
            preview: { rowCount: ids.length, plan: ids },
            transaction,
            write: (tx, plan) => {
              plan.forEach((id, step) => {
                if (step === failAt) throw new Error('write failed');
                tx.set(id, 'imported');
              });
              return Promise.resolve();
            },
          });
          if (failAt < ids.length) {
            await expect(outcome).rejects.toThrow('write failed');
            expect(committed).toEqual(before);
          } else await expect(outcome).resolves.toBeUndefined();
        },
      ),
    );
  });

  it('eraseProductProfile signs out iff the server erase succeeded', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.constantFrom('ok', 'fail', 'ambiguous', 'hook-fail'),
        fc.boolean(),
        async (server, localFails) => {
          const events: string[] = [];
          await eraseProductProfile({
            beforeServerErase: [
              () =>
                server === 'hook-fail' ? Promise.reject(new Error('flush')) : Promise.resolve(),
            ],
            eraseServerProfile: () => {
              events.push('server');
              if (server === 'fail') return Promise.reject(new Error('down'));
              if (server === 'ambiguous') {
                return Promise.reject(new AmbiguousProductProfileErasureError());
              }
              return Promise.resolve({ operationId: 'op', status: 'completed' });
            },
            eraseLocalPartition: () => {
              events.push('local');
              return localFails ? Promise.reject(new Error('disk')) : Promise.resolve();
            },
            signOut: () => {
              events.push('sign-out');
              return Promise.resolve();
            },
          });
          const succeeded = server === 'ok';
          expect(events.includes('sign-out')).toBe(succeeded);
          expect(events.includes('local')).toBe(succeeded);
          expect(events.includes('server')).toBe(server !== 'hook-fail');
        },
      ),
    );
  });
});
