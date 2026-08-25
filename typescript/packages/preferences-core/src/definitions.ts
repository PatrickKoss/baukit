export type PreferenceScope = 'device' | 'identity' | 'synced';

export type PreferenceSideEffectFailurePolicy = 'ignore' | 'report';

export interface PreferenceSideEffectContext<TKey extends string, TValue, TValues extends object> {
  readonly key: TKey;
  readonly previousValue: TValue;
  readonly value: TValue;
  readonly previousValues: TValues;
  readonly values: TValues;
}

export interface AfterPersistencePreferenceSideEffect<
  TKey extends string,
  TValue,
  TValues extends object,
> {
  readonly mode: 'after-persistence';
  readonly onError: PreferenceSideEffectFailurePolicy;
  readonly run: (
    context: PreferenceSideEffectContext<TKey, TValue, TValues>,
  ) => Promise<void> | void;
}

export interface PreviewPreferenceSideEffect<TKey extends string, TValue, TValues extends object> {
  readonly mode: 'preview-with-rollback';
  readonly onError: PreferenceSideEffectFailurePolicy;
  readonly preview: (
    context: PreferenceSideEffectContext<TKey, TValue, TValues>,
  ) => Promise<void> | void;
  readonly rollback: (
    context: PreferenceSideEffectContext<TKey, TValue, TValues>,
  ) => Promise<void> | void;
  readonly afterPersistence?: (
    context: PreferenceSideEffectContext<TKey, TValue, TValues>,
  ) => Promise<void> | void;
}

export type PreferenceSideEffect<TKey extends string, TValue, TValues extends object> =
  | AfterPersistencePreferenceSideEffect<TKey, TValue, TValues>
  | PreviewPreferenceSideEffect<TKey, TValue, TValues>;

export interface PreferenceDefinition<
  TKey extends string,
  TValue,
  TValues extends object = Record<TKey, TValue>,
> {
  readonly key: TKey;
  readonly defaultValue: TValue;
  readonly normalize: (value: unknown) => TValue;
  readonly scope: PreferenceScope;
  readonly sideEffect?: PreferenceSideEffect<TKey, TValue, TValues>;
}

export type PreferenceKey<TValues extends object> = Extract<keyof TValues, string>;

export type PreferenceRegistry<TValues extends object> = {
  readonly [TKey in PreferenceKey<TValues>]: PreferenceDefinition<TKey, TValues[TKey], TValues>;
};

export function definePreferenceRegistry<TValues extends object>(
  registry: PreferenceRegistry<TValues>,
): PreferenceRegistry<TValues> {
  return registry;
}

export function normalizePreferences<TValues extends object>(
  definitions: PreferenceRegistry<TValues>,
  input: Partial<TValues> | undefined,
): TValues {
  const normalized: Record<string, unknown> = {};
  for (const key of Object.keys(definitions) as PreferenceKey<TValues>[]) {
    const definition = definitions[key];
    const value = input && Object.hasOwn(input, key) ? input[key] : definition.defaultValue;
    normalized[key] = definition.normalize(value);
  }
  return normalized as TValues;
}
