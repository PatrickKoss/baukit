export type ImportEnvelopeSource = string | Uint8Array;

export interface ImportEnvelopeLimits {
  readonly maxSourceBytes: number;
  readonly maxRows: number;
  readonly maxStringBytes: number;
}

export type ImportEnvelopeErrorCode =
  | 'source_too_large'
  | 'too_many_rows'
  | 'collection_not_allowed'
  | 'field_not_allowed'
  | 'string_too_large'
  | 'invalid_row_value';

export interface ImportEnvelopeErrorDetails {
  readonly collection?: string;
  readonly rowIndex?: number;
  readonly field?: string;
  readonly measured?: number;
  readonly allowed?: number;
}

export class ImportEnvelopeError extends Error {
  public override readonly name = 'ImportEnvelopeError';
  public readonly code: ImportEnvelopeErrorCode;
  public readonly collection: string | undefined;
  public readonly rowIndex: number | undefined;
  public readonly field: string | undefined;
  public readonly measured: number | undefined;
  public readonly allowed: number | undefined;

  public constructor(code: ImportEnvelopeErrorCode, details: ImportEnvelopeErrorDetails = {}) {
    super(`Import envelope rejected: ${code}`);
    this.code = code;
    this.collection = details.collection;
    this.rowIndex = details.rowIndex;
    this.field = details.field;
    this.measured = details.measured;
    this.allowed = details.allowed;
  }
}

export interface DecodedImportEnvelopeRow<TCollection extends string> {
  readonly collection: TCollection;
  readonly fields: Readonly<Record<string, unknown>>;
}

export interface DecodedImportEnvelope<TContext, TCollection extends string> {
  readonly context: TContext;
  readonly rows: Iterable<DecodedImportEnvelopeRow<TCollection>>;
}

export interface ImportEnvelopeRowDecoderInput<TContext, TCollection extends string> {
  readonly context: TContext;
  readonly collection: TCollection;
  readonly rowIndex: number;
  readonly fields: Readonly<Record<string, unknown>>;
}

export interface ImportEnvelopePlannerInput<TContext, TRow> {
  readonly context: TContext;
  readonly rows: readonly TRow[];
}

export interface ImportEnvelopePreview<TPlan> {
  readonly rowCount: number;
  readonly plan: TPlan;
}

export interface PrepareImportEnvelopeOptions<TContext, TCollection extends string, TRow, TPlan> {
  readonly source: ImportEnvelopeSource;
  readonly limits: ImportEnvelopeLimits;
  readonly decodeEnvelope: (
    source: ImportEnvelopeSource,
  ) =>
    | DecodedImportEnvelope<TContext, TCollection>
    | Promise<DecodedImportEnvelope<TContext, TCollection>>;
  readonly fieldAllowlist: Readonly<Partial<Record<TCollection, readonly string[]>>>;
  readonly decodeRow: (
    input: ImportEnvelopeRowDecoderInput<TContext, TCollection>,
  ) => TRow | Promise<TRow>;
  readonly plan: (input: ImportEnvelopePlannerInput<TContext, TRow>) => TPlan | Promise<TPlan>;
}

export interface ImportEnvelopeTransactionAdapter<TTransaction> {
  withTransaction<TResult>(
    operation: (transaction: TTransaction) => Promise<TResult>,
  ): Promise<TResult>;
}

export interface CommitImportEnvelopeOptions<TPlan, TTransaction, TResult> {
  readonly preview: ImportEnvelopePreview<TPlan>;
  readonly transaction: ImportEnvelopeTransactionAdapter<TTransaction>;
  readonly write: (transaction: TTransaction, plan: TPlan) => Promise<TResult>;
  readonly afterCommit?: (result: TResult) => Promise<void> | void;
}

export async function prepareImportEnvelope<TContext, TCollection extends string, TRow, TPlan>(
  options: PrepareImportEnvelopeOptions<TContext, TCollection, TRow, TPlan>,
): Promise<ImportEnvelopePreview<TPlan>> {
  assertLimits(options.limits);
  const sourceBytes = utf8ByteLength(options.source);
  if (sourceBytes > options.limits.maxSourceBytes) {
    throw new ImportEnvelopeError('source_too_large', {
      measured: sourceBytes,
      allowed: options.limits.maxSourceBytes,
    });
  }

  const envelope = await options.decodeEnvelope(options.source);
  const decodedRows: TRow[] = [];
  for (const row of envelope.rows) {
    const rowIndex = decodedRows.length;
    if (rowIndex >= options.limits.maxRows) {
      throw new ImportEnvelopeError('too_many_rows', {
        measured: rowIndex + 1,
        allowed: options.limits.maxRows,
      });
    }
    const allowedFields = allowlistFor(options.fieldAllowlist, row.collection, rowIndex);
    const fields = sanitizeFields(
      row.fields,
      allowedFields,
      row.collection,
      rowIndex,
      options.limits.maxStringBytes,
    );
    decodedRows.push(
      await options.decodeRow({
        context: envelope.context,
        collection: row.collection,
        rowIndex,
        fields,
      }),
    );
  }

  const rows = Object.freeze(decodedRows.slice());
  const plan = await options.plan({ context: envelope.context, rows });
  return Object.freeze({ rowCount: rows.length, plan });
}

export async function commitImportEnvelope<TPlan, TTransaction, TResult>(
  options: CommitImportEnvelopeOptions<TPlan, TTransaction, TResult>,
): Promise<TResult> {
  const result = await options.transaction.withTransaction((transaction) =>
    options.write(transaction, options.preview.plan),
  );
  await options.afterCommit?.(result);
  return result;
}

function assertLimits(limits: ImportEnvelopeLimits): void {
  assertLimit(limits.maxSourceBytes, 'maxSourceBytes');
  assertLimit(limits.maxRows, 'maxRows');
  assertLimit(limits.maxStringBytes, 'maxStringBytes');
}

function assertLimit(value: number, name: keyof ImportEnvelopeLimits): void {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${name} must be a non-negative safe integer`);
  }
}

function allowlistFor<TCollection extends string>(
  allowlist: Readonly<Partial<Record<TCollection, readonly string[]>>>,
  collection: TCollection,
  rowIndex: number,
): ReadonlySet<string> {
  const fields = (allowlist as Readonly<Record<string, readonly string[] | undefined>>)[collection];
  if (fields === undefined) {
    throw new ImportEnvelopeError('collection_not_allowed', { collection, rowIndex });
  }
  return new Set(fields);
}

function sanitizeFields(
  fields: Readonly<Record<string, unknown>>,
  allowedFields: ReadonlySet<string>,
  collection: string,
  rowIndex: number,
  maxStringBytes: number,
): Readonly<Record<string, unknown>> {
  if (Object.getOwnPropertySymbols(fields).length > 0) {
    throw new ImportEnvelopeError('invalid_row_value', { collection, rowIndex });
  }
  const entries: [string, unknown][] = [];
  for (const field of Object.keys(fields)) {
    if (!allowedFields.has(field)) {
      throw new ImportEnvelopeError('field_not_allowed', { collection, rowIndex, field });
    }
    const descriptor = Object.getOwnPropertyDescriptor(fields, field);
    if (descriptor === undefined || !('value' in descriptor)) {
      throw new ImportEnvelopeError('invalid_row_value', { collection, rowIndex, field });
    }
    assertBoundedStrings(
      descriptor.value,
      maxStringBytes,
      { collection, rowIndex, field },
      new WeakSet(),
    );
    entries.push([field, descriptor.value]);
  }
  return Object.freeze(Object.fromEntries(entries));
}

function assertBoundedStrings(
  value: unknown,
  allowed: number,
  location: Required<Pick<ImportEnvelopeErrorDetails, 'collection' | 'rowIndex' | 'field'>>,
  ancestors: WeakSet<object>,
): void {
  if (typeof value === 'string') {
    const measured = utf8StringBytes(value);
    if (measured > allowed) {
      throw new ImportEnvelopeError('string_too_large', { ...location, measured, allowed });
    }
    return;
  }
  if (value === null || typeof value !== 'object') return;
  if (ancestors.has(value)) {
    throw new ImportEnvelopeError('invalid_row_value', location);
  }

  ancestors.add(value);
  try {
    for (const key of Reflect.ownKeys(value)) {
      if (typeof key !== 'string') {
        throw new ImportEnvelopeError('invalid_row_value', location);
      }
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (descriptor === undefined || !('value' in descriptor)) {
        throw new ImportEnvelopeError('invalid_row_value', location);
      }
      assertBoundedStrings(descriptor.value, allowed, location, ancestors);
    }
  } finally {
    ancestors.delete(value);
  }
}

function utf8ByteLength(source: ImportEnvelopeSource): number {
  return typeof source === 'string' ? utf8StringBytes(source) : source.byteLength;
}

function utf8StringBytes(value: string): number {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const first = value.charCodeAt(index);
    if (first <= 0x7f) bytes += 1;
    else if (first <= 0x7ff) bytes += 2;
    else if (first >= 0xd800 && first <= 0xdbff && isLowSurrogate(value.charCodeAt(index + 1))) {
      bytes += 4;
      index += 1;
    } else bytes += 3;
  }
  return bytes;
}

function isLowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}
