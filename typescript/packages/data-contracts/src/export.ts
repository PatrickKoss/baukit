export interface CsvNumericCell {
  readonly numeric: string;
}

export type CsvCell = string | number | CsvNumericCell | null;

export interface EncodeCsvOptions {
  readonly neutralizeFormulas?: boolean;
  readonly byteOrderMark?: boolean;
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
  | { readonly kind: 'text'; readonly value: string }
  | { readonly kind: 'numeric'; readonly value: string };

export function csvNumeric(value: string): CsvNumericCell {
  return { numeric: value };
}

export function encodeCsv(
  rows: Iterable<readonly CsvCell[]>,
  options: EncodeCsvOptions = {},
): string {
  const neutralizeFormulas = options.neutralizeFormulas ?? true;
  let output = options.byteOrderMark === true ? BYTE_ORDER_MARK : '';
  let rowIndex = 0;
  for (const row of rows) {
    output += encodeRecord(row, rowIndex, neutralizeFormulas) + RECORD_SEPARATOR;
    rowIndex += 1;
  }
  return output;
}

function encodeRecord(row: readonly CsvCell[], rowIndex: number, neutralize: boolean): string {
  if (row.length === 1 && isEmptyCell(row[0])) return '""';
  return row
    .map((cell, columnIndex) => writeCell(classifyCell(cell, rowIndex, columnIndex), neutralize))
    .join(',');
}

function isEmptyCell(cell: CsvCell | undefined): boolean {
  return cell === null || cell === '';
}

function classifyCell(cell: CsvCell, rowIndex: number, columnIndex: number): EncodedCell {
  if (cell === null) return { kind: 'text', value: '' };
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

function writeCell(cell: EncodedCell, neutralize: boolean): string {
  if (cell.kind === 'numeric') return cell.value;
  const text =
    neutralize && startsLikeFormula(cell.value) ? FORMULA_ESCAPE + cell.value : cell.value;
  return QUOTE_REQUIRED_PATTERN.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function startsLikeFormula(value: string): boolean {
  const first = value.charAt(0);
  if (CONTROL_TRIGGERS.has(first)) return true;
  const afterWhitespace = value.replace(LEADING_WHITESPACE_PATTERN, '');
  return FORMULA_TRIGGERS.has(afterWhitespace.charAt(0));
}
