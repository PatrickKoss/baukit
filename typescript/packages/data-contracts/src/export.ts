export interface CsvNumericCell {
  readonly numeric: string;
}

export type CsvCell = string | number | CsvNumericCell | null;

export interface EncodeCsvOptions {
  readonly neutralizeFormulas?: boolean;
  readonly byteOrderMark?: boolean;
  /** Quotes every text and numeric cell. Null cells stay unquoted, so a reader can tell them from empty text. */
  readonly quoteAllCells?: boolean;
  /**
   * Written unquoted for a null cell; a text cell with the same content is quoted. Must be non-empty
   * and free of double quotes, commas, CR, and LF.
   */
  readonly nullMarker?: string;
}

export type CsvEncodeErrorCode = 'invalid_numeric_cell' | 'invalid_unicode' | 'unsupported_cell';

export class CsvEncodeError extends Error {
  public override readonly name = 'CsvEncodeError';
  public readonly code: CsvEncodeErrorCode;
  public readonly rowIndex: number;
  public readonly columnIndex: number;

  public constructor(code: CsvEncodeErrorCode, rowIndex: number, columnIndex: number) {
    super(`CSV cell rejected: ${code} at row ${String(rowIndex)}, column ${String(columnIndex)}`);
    this.code = code;
    this.rowIndex = rowIndex;
    this.columnIndex = columnIndex;
  }
}

export const SHARE_OUTCOMES = ['shared', 'saved', 'cancelled', 'unavailable', 'failed'] as const;

export type ShareOutcome = (typeof SHARE_OUTCOMES)[number];

const BYTE_ORDER_MARK_CODE_POINT = 0xfeff;
const BYTE_ORDER_MARK = String.fromCodePoint(BYTE_ORDER_MARK_CODE_POINT);
const RECORD_SEPARATOR = '\r\n';
const FORMULA_ESCAPE = "'";
const NUMERIC_PATTERN = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$/;
const QUOTE_REQUIRED_PATTERN = /[",\r\n]/;
const HIGH_SURROGATE_MIN = 0xd800;
const HIGH_SURROGATE_MAX = 0xdbff;
const LOW_SURROGATE_MIN = 0xdc00;
const LOW_SURROGATE_MAX = 0xdfff;
const LEADING_WHITESPACE_PATTERN = /^[ \t\r\n]*/;
const FORMULA_TRIGGERS = new Set(['=', '+', '-', '@']);
const CONTROL_TRIGGERS = new Set(['\t', '\r']);

type EncodedCell =
  | { readonly kind: 'null' }
  | { readonly kind: 'text'; readonly value: string }
  | { readonly kind: 'numeric'; readonly value: string };

interface WritePolicy {
  readonly neutralizeFormulas: boolean;
  readonly quoteAllCells: boolean;
  readonly nullMarker: string | undefined;
}

export function csvNumeric(value: string): CsvNumericCell {
  return { numeric: value };
}

export function encodeCsv(
  rows: Iterable<readonly CsvCell[]>,
  options: EncodeCsvOptions = {},
): string {
  const policy = writePolicy(options);
  let output = options.byteOrderMark === true ? BYTE_ORDER_MARK : '';
  let rowIndex = 0;
  for (const row of rows) {
    output += encodeRecord(row, rowIndex, policy) + RECORD_SEPARATOR;
    rowIndex += 1;
  }
  return output;
}

function writePolicy(options: EncodeCsvOptions): WritePolicy {
  const { nullMarker } = options;
  if (nullMarker !== undefined && (nullMarker === '' || QUOTE_REQUIRED_PATTERN.test(nullMarker))) {
    throw new RangeError(
      'CSV null marker must be non-empty and free of double quotes, commas, CR, and LF.',
    );
  }
  return {
    neutralizeFormulas: options.neutralizeFormulas ?? true,
    quoteAllCells: options.quoteAllCells ?? false,
    nullMarker,
  };
}

function encodeRecord(row: readonly CsvCell[], rowIndex: number, policy: WritePolicy): string {
  const record = row
    .map((cell, columnIndex) => writeCell(classifyCell(cell, rowIndex, columnIndex), policy))
    .join(',');
  return row.length === 1 && record === '' ? '""' : record;
}

function classifyCell(cell: CsvCell, rowIndex: number, columnIndex: number): EncodedCell {
  if (cell === null) return { kind: 'null' };
  if (typeof cell === 'string') {
    if (hasUnpairedSurrogate(cell)) {
      throw new CsvEncodeError('invalid_unicode', rowIndex, columnIndex);
    }
    return { kind: 'text', value: cell };
  }
  const numeric = numericText(cell);
  if (numeric === undefined) throw new CsvEncodeError('unsupported_cell', rowIndex, columnIndex);
  if (!NUMERIC_PATTERN.test(numeric)) {
    throw new CsvEncodeError('invalid_numeric_cell', rowIndex, columnIndex);
  }
  return { kind: 'numeric', value: numeric };
}

function hasUnpairedSurrogate(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (isInRange(unit, LOW_SURROGATE_MIN, LOW_SURROGATE_MAX)) return true;
    if (!isInRange(unit, HIGH_SURROGATE_MIN, HIGH_SURROGATE_MAX)) continue;
    if (!isInRange(value.charCodeAt(index + 1), LOW_SURROGATE_MIN, LOW_SURROGATE_MAX)) return true;
    index += 1;
  }
  return false;
}

function isInRange(value: number, min: number, max: number): boolean {
  return value >= min && value <= max;
}

function numericText(cell: unknown): string | undefined {
  if (typeof cell === 'number') return String(cell);
  if (typeof cell !== 'object' || cell === null || !('numeric' in cell)) return undefined;
  const { numeric } = cell;
  return typeof numeric === 'string' ? numeric : undefined;
}

function writeCell(cell: EncodedCell, policy: WritePolicy): string {
  if (cell.kind === 'null') return policy.nullMarker ?? '';
  if (cell.kind === 'numeric') return policy.quoteAllCells ? quote(cell.value) : cell.value;
  const text =
    policy.neutralizeFormulas && startsLikeFormula(cell.value)
      ? FORMULA_ESCAPE + cell.value
      : cell.value;
  return mustQuote(text, policy) ? quote(text) : text;
}

function mustQuote(text: string, policy: WritePolicy): boolean {
  return policy.quoteAllCells || text === policy.nullMarker || QUOTE_REQUIRED_PATTERN.test(text);
}

function quote(text: string): string {
  return `"${text.replaceAll('"', '""')}"`;
}

function startsLikeFormula(value: string): boolean {
  const first = value.charAt(0);
  if (CONTROL_TRIGGERS.has(first)) return true;
  const afterWhitespace = value.replace(LEADING_WHITESPACE_PATTERN, '');
  return FORMULA_TRIGGERS.has(afterWhitespace.charAt(0));
}
