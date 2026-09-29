import { describe, expect, it } from 'vitest';

import fixtureCorpus from '../../../../fixtures/export-csv/csv-encoding-v1.json' with { type: 'json' };

import {
  CsvEncodeError,
  SHARE_OUTCOMES,
  type CsvCell,
  type EncodeCsvOptions,
  type ShareOutcome,
  csvNumeric,
  encodeCsv,
} from './export.js';

type FixtureCell = string | null | { readonly numeric: string };

interface FixtureCase {
  readonly name: string;
  readonly options?: EncodeCsvOptions;
  readonly rows: readonly (readonly FixtureCell[])[];
}

interface FixtureCorpus {
  readonly version: number;
  readonly byteOrderMarkCodePoint: number;
  readonly cases: readonly (FixtureCase & { readonly csv: string })[];
  readonly rejections: readonly (FixtureCase & {
    readonly error: {
      readonly code: string;
      readonly rowIndex: number;
      readonly columnIndex: number;
    };
  })[];
}

const corpus = fixtureCorpus as FixtureCorpus;
const byteOrderMark = String.fromCodePoint(corpus.byteOrderMarkCodePoint);

function expectedOutput(csv: string, options: EncodeCsvOptions | undefined): string {
  return options?.byteOrderMark === true ? byteOrderMark + csv : csv;
}

function rejection(rows: readonly (readonly CsvCell[])[]): CsvEncodeError {
  try {
    encodeCsv(rows);
  } catch (error) {
    if (error instanceof CsvEncodeError) return error;
    throw error;
  }
  throw new Error('Expected encodeCsv to reject');
}

describe('shared CSV vectors', () => {
  it('uses version 1 and covers every case group', () => {
    expect(corpus.version).toBe(1);
    expect(corpus.cases.length).toBeGreaterThan(0);
    expect(corpus.rejections.length).toBeGreaterThan(0);
    expect(corpus.byteOrderMarkCodePoint).toBe(0xfeff);
  });

  it.each(corpus.cases)('encodes $name', ({ rows, options, csv }) => {
    expect(encodeCsv(rows, options)).toBe(expectedOutput(csv, options));
  });

  it('starts with U+FEFF only when the byte-order mark is requested', () => {
    expect(encodeCsv([['a']], { byteOrderMark: true }).codePointAt(0)).toBe(0xfeff);
    expect(encodeCsv([['a']]).codePointAt(0)).toBe('a'.codePointAt(0));
  });

  it.each(corpus.rejections)('rejects $name', ({ rows, options, error }) => {
    expect(() => encodeCsv(rows, options)).toThrow(
      expect.objectContaining({
        name: 'CsvEncodeError',
        code: error.code,
        rowIndex: error.rowIndex,
        columnIndex: error.columnIndex,
      }),
    );
  });
});

describe('encodeCsv', () => {
  it('writes finite numbers without formula neutralization', () => {
    expect(encodeCsv([[-5, 0, -0, 1.5, 1e21, csvNumeric('-2.50')]])).toBe(
      '-5,0,0,1.5,1e+21,-2.50\r\n',
    );
  });

  it.each([Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
    'rejects the non-finite number %s',
    (value) => {
      expect(rejection([['ok', value]])).toMatchObject({
        code: 'invalid_numeric_cell',
        rowIndex: 0,
        columnIndex: 1,
      });
    },
  );

  const highSurrogate = String.fromCharCode(0xd800);
  const lowSurrogate = String.fromCharCode(0xdc00);

  it.each([
    ['a lone high surrogate', `${highSurrogate}secret`],
    ['a lone low surrogate', `secret${lowSurrogate}`],
    ['a trailing high surrogate', `secret${highSurrogate}`],
    ['reversed surrogates', `${lowSurrogate}${highSurrogate}secret`],
  ])('rejects %s without echoing the cell value', (_label, value) => {
    const error = rejection([['ok'], [value]]);
    expect(error).toMatchObject({ code: 'invalid_unicode', rowIndex: 1, columnIndex: 0 });
    expect(error.message).not.toContain('secret');
  });

  it('keeps paired surrogates', () => {
    expect(encodeCsv([['😀']])).toBe('😀\r\n');
  });

  it.each([undefined, true, {}, { numeric: 5 }])(
    'rejects the unsupported runtime cell %s',
    (value) => {
      expect(rejection([[value as unknown as CsvCell]])).toMatchObject({
        code: 'unsupported_cell',
      });
    },
  );

  it('does not echo rejected numeric text', () => {
    expect(rejection([[csvNumeric('=secret()')]]).message).not.toContain('secret');
  });

  it.each(['', 'a,b', 'say "null"', 'line\n', 'line\r'])(
    'rejects the null marker %j before writing',
    (nullMarker) => {
      expect(() => encodeCsv([[null]], { nullMarker })).toThrow(RangeError);
    },
  );

  it('quotes a neutralized cell that would read as the null marker', () => {
    expect(encodeCsv([[null, '-']], { nullMarker: "'-" })).toBe(`'-,"'-"\r\n`);
  });

  it('accepts any iterable of rows', () => {
    function* rows(): Generator<readonly CsvCell[]> {
      yield ['id'];
      yield ['=1'];
    }
    expect(encodeCsv(rows())).toBe("id\r\n'=1\r\n");
  });
});

describe('share outcome', () => {
  it('lists every outcome once', () => {
    const outcomes: readonly ShareOutcome[] = SHARE_OUTCOMES;
    expect(outcomes).toEqual(['shared', 'saved', 'cancelled', 'unavailable', 'failed']);
  });
});

it('exports the encoder from the package root and the export entry point', async () => {
  const [root, entry] = await Promise.all([
    import('@baukit/data-contracts'),
    import('@baukit/data-contracts/export'),
  ]);
  expect(root.encodeCsv).toBe(entry.encodeCsv);
  expect(entry.CsvEncodeError).toBeTypeOf('function');
  expect(entry.SHARE_OUTCOMES).toContain('cancelled');
  expect(root.prepareImportEnvelope).toBeTypeOf('function');
});
