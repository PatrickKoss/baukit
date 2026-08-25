export interface PreferenceStore<TValues extends object> {
  read(): Promise<TValues | undefined>;
  patch(patch: Partial<TValues>): Promise<TValues>;
}

/**
 * The slice of a record-keyed repository this package needs. Products usually already
 * have a repository of this shape for the row that holds a subject's settings.
 *
 * @typeParam TSubjectId - identifier the repository is keyed by, such as a user id.
 * @typeParam TRecord - the stored record.
 * @typeParam TRecordPatch - the partial record the repository accepts on write.
 */
export interface PreferenceRecordRepository<TSubjectId, TRecord, TRecordPatch> {
  get(subjectId: TSubjectId): Promise<TRecord | null | undefined>;
  upsert(subjectId: TSubjectId, patch: TRecordPatch): Promise<TRecord>;
}

export interface RepositoryPreferenceStoreOptions<
  TValues extends object,
  TSubjectId,
  TRecord,
  TRecordPatch,
> {
  readonly repository: PreferenceRecordRepository<TSubjectId, TRecord, TRecordPatch>;
  readonly subjectId: TSubjectId;
  /** Projects a stored record onto preference values. */
  readonly toValues: (record: TRecord) => TValues;
  /** Projects a preference patch onto the record patch the repository accepts. */
  readonly toRecordPatch: (patch: Partial<TValues>) => TRecordPatch;
}

/**
 * Adapts a record-keyed repository to {@link PreferenceStore}. It owns the mapping
 * between preference values and stored columns, so the controller never sees the
 * record shape and the repository never sees preference keys.
 */
export class RepositoryPreferenceStore<
  TValues extends object,
  TSubjectId,
  TRecord,
  TRecordPatch,
> implements PreferenceStore<TValues> {
  readonly #repository: PreferenceRecordRepository<TSubjectId, TRecord, TRecordPatch>;
  readonly #subjectId: TSubjectId;
  readonly #toValues: (record: TRecord) => TValues;
  readonly #toRecordPatch: (patch: Partial<TValues>) => TRecordPatch;

  constructor(
    options: RepositoryPreferenceStoreOptions<TValues, TSubjectId, TRecord, TRecordPatch>,
  ) {
    this.#repository = options.repository;
    this.#subjectId = options.subjectId;
    this.#toValues = options.toValues;
    this.#toRecordPatch = options.toRecordPatch;
  }

  async read(): Promise<TValues | undefined> {
    const record = await this.#repository.get(this.#subjectId);
    return record == null ? undefined : this.#toValues(record);
  }

  async patch(patch: Partial<TValues>): Promise<TValues> {
    const record = await this.#repository.upsert(this.#subjectId, this.#toRecordPatch(patch));
    return this.#toValues(record);
  }
}

export function createRepositoryPreferenceStore<
  TValues extends object,
  TSubjectId,
  TRecord,
  TRecordPatch,
>(
  options: RepositoryPreferenceStoreOptions<TValues, TSubjectId, TRecord, TRecordPatch>,
): PreferenceStore<TValues> {
  return new RepositoryPreferenceStore(options);
}

export class InMemoryPreferenceStore<TValues extends object> implements PreferenceStore<TValues> {
  readonly #defaults: TValues;
  #values: TValues | undefined;

  constructor(defaults: TValues, initialValues?: TValues) {
    this.#defaults = { ...defaults };
    this.#values = initialValues ? { ...initialValues } : undefined;
  }

  read(): Promise<TValues | undefined> {
    return Promise.resolve(this.#values ? { ...this.#values } : undefined);
  }

  patch(patch: Partial<TValues>): Promise<TValues> {
    this.#values = { ...this.#defaults, ...this.#values, ...patch };
    return Promise.resolve({ ...this.#values });
  }
}
