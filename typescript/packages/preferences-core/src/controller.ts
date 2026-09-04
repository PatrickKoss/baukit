import {
  normalizePreferences,
  type PreferenceKey,
  type PreferenceRegistry,
  type PreferenceSideEffectContext,
} from './definitions.js';
import type { PreferenceStore } from './store.js';
import type { OptionalWireValue } from './wire.js';

export type PreferenceControllerStatus = 'idle' | 'hydrating' | 'ready';

export interface PreferenceControllerState<TValues extends object> {
  readonly values: TValues;
  readonly status: PreferenceControllerStatus;
  readonly error: Error | null;
}

export interface SerializedPreferenceControllerState<
  TValues extends object,
> extends PreferenceControllerState<TValues> {
  readonly pendingCount: number;
}

export interface PreferenceController<TValues extends object> {
  getState(): PreferenceControllerState<TValues>;
  hydrate(): Promise<TValues>;
  update(patch: Partial<TValues>): Promise<TValues>;
  applyWireValue<TKey extends PreferenceKey<TValues>>(
    key: TKey,
    value: OptionalWireValue<Exclude<TValues[TKey], null>>,
  ): Promise<TValues>;
  switchIdentity(store: PreferenceStore<TValues>): Promise<TValues>;
  /**
   * Stops publishing to `onVisibleChange`. Call it when the consumer that owns the
   * callback goes away, such as a React provider unmounting. In-flight reads and
   * writes still settle and still update `getState()`; they just stop being
   * published. Stopping is permanent for this controller instance.
   */
  stop(): void;
}

export interface SerializedPreferenceController<
  TValues extends object,
> extends PreferenceController<TValues> {
  getState(): SerializedPreferenceControllerState<TValues>;
}

export type PreferenceUpdateMode = 'optimistic' | 'serialized';

export interface CreatePreferenceControllerOptions<TValues extends object> {
  readonly definitions: PreferenceRegistry<TValues>;
  readonly store: PreferenceStore<TValues>;
  readonly onVisibleChange: (values: TValues) => void;
  readonly updateMode?: PreferenceUpdateMode;
}

class DefaultPreferenceController<TValues extends object> implements PreferenceController<TValues> {
  readonly #definitions: PreferenceRegistry<TValues>;
  readonly #onVisibleChange: (values: TValues) => void;
  readonly #updateMode: PreferenceUpdateMode;
  #store: PreferenceStore<TValues>;
  #state: PreferenceControllerState<TValues>;
  #identityRevision = 0;
  #pendingCount = 0;
  #updateTail: Promise<void> = Promise.resolve();
  #stopped = false;

  constructor(options: CreatePreferenceControllerOptions<TValues>) {
    this.#definitions = options.definitions;
    this.#store = options.store;
    this.#onVisibleChange = options.onVisibleChange;
    this.#updateMode = options.updateMode ?? 'optimistic';
    this.#state = this.#withPendingCount({
      values: normalizePreferences(this.#definitions, undefined),
      status: 'idle',
      error: null,
    });
  }

  getState(): PreferenceControllerState<TValues> {
    return this.#state;
  }

  async hydrate(): Promise<TValues> {
    const revision = this.#identityRevision;
    this.#setState({ ...this.#state, status: 'hydrating', error: null });
    try {
      const stored = await this.#store.read();
      const values = normalizePreferences(this.#definitions, stored);
      if (!this.#setVisible(values, 'ready', null, revision)) {
        return this.#state.values;
      }
      return values;
    } catch (error) {
      const values = normalizePreferences(this.#definitions, undefined);
      this.#setVisible(values, 'ready', asError(error), revision);
      throw asError(error);
    }
  }

  update(patch: Partial<TValues>): Promise<TValues> {
    if (this.#updateMode === 'serialized') {
      let normalizedPatch: Partial<TValues>;
      try {
        normalizedPatch = this.#normalizePatch(patch);
      } catch (error) {
        return Promise.reject(asError(error));
      }
      return this.#enqueueSerializedUpdate(normalizedPatch);
    }
    return this.#runUpdate(patch, this.#identityRevision, this.#store, true);
  }

  async #runUpdate(
    patch: Partial<TValues>,
    revision: number,
    store: PreferenceStore<TValues>,
    optimistic: boolean,
  ): Promise<TValues> {
    if (revision !== this.#identityRevision) {
      return this.#state.values;
    }
    const previousValues = this.#state.values;
    const normalizedPatch = optimistic ? this.#normalizePatch(patch) : patch;
    const values = { ...previousValues, ...normalizedPatch };
    const changedKeys = this.#changedKeys(previousValues, values, normalizedPatch);

    if (optimistic) {
      this.#setVisible(values, 'ready', null, revision);
    }
    const previewed: PreferenceKey<TValues>[] = [];
    let persisted: TValues;
    try {
      for (const key of changedKeys) {
        const effect = this.#definitions[key].sideEffect;
        if (effect?.mode === 'preview-with-rollback') {
          previewed.push(key);
          try {
            await effect.preview(this.#effectContext(key, previousValues, values));
          } catch (error) {
            if (effect.onError === 'report') {
              throw error;
            }
          }
        }
      }
      persisted = normalizePreferences(this.#definitions, await store.patch(normalizedPatch));
    } catch (error) {
      const updateError = asError(error);
      const rollbackError = await this.#rollbackPreviews(previewed, previousValues, values);
      const reportedError = rollbackError
        ? new AggregateError(
            [updateError, rollbackError],
            'Preference update and preview rollback failed',
          )
        : updateError;
      if (optimistic) {
        this.#setVisible(previousValues, 'ready', reportedError, revision);
      }
      throw reportedError;
    }
    if (!this.#setVisible(persisted, 'ready', null, revision)) {
      await this.#rollbackPreviews(previewed, previousValues, values);
      return this.#state.values;
    }
    await this.#runPersistedEffects(changedKeys, previousValues, persisted, revision);
    return persisted;
  }

  #enqueueSerializedUpdate(patch: Partial<TValues>): Promise<TValues> {
    const revision = this.#identityRevision;
    const store = this.#store;
    const startImmediately = this.#pendingCount === 0;
    this.#pendingCount += 1;
    this.#setVisible(this.#state.values, this.#state.status, null, revision);

    const run = () => this.#runUpdate(patch, revision, store, false);
    const write = startImmediately ? run() : this.#updateTail.then(run);
    const operation = write.then(
      (values) => {
        this.#finishSerializedUpdate(revision, null);
        return values;
      },
      (reason: unknown) => {
        const error = asError(reason);
        this.#finishSerializedUpdate(revision, error);
        throw error;
      },
    );
    this.#updateTail = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }

  #finishSerializedUpdate(revision: number, error: Error | null): void {
    if (revision !== this.#identityRevision) {
      return;
    }
    this.#pendingCount = Math.max(0, this.#pendingCount - 1);
    this.#setVisible(this.#state.values, 'ready', error, revision);
  }

  applyWireValue<TKey extends PreferenceKey<TValues>>(
    key: TKey,
    value: OptionalWireValue<Exclude<TValues[TKey], null>>,
  ): Promise<TValues> {
    if (value.state === 'absent') {
      return Promise.resolve(this.#state.values);
    }
    const patch: Record<string, unknown> = {
      [key]: value.state === 'null' ? null : value.value,
    };
    return this.update(patch as Partial<TValues>);
  }

  switchIdentity(store: PreferenceStore<TValues>): Promise<TValues> {
    this.#identityRevision += 1;
    this.#pendingCount = 0;
    this.#updateTail = Promise.resolve();
    this.#store = store;
    const defaults = normalizePreferences(this.#definitions, undefined);
    this.#setVisible(defaults, 'idle', null);
    return this.hydrate();
  }

  stop(): void {
    this.#stopped = true;
  }

  #setState(state: PreferenceControllerState<TValues>): void {
    this.#state = this.#withPendingCount(state);
  }

  #setVisible(
    values: TValues,
    status: PreferenceControllerStatus,
    error: Error | null,
    revision: number = this.#identityRevision,
  ): boolean {
    if (revision !== this.#identityRevision) {
      return false;
    }
    this.#state = this.#withPendingCount({ values, status, error });
    if (!this.#stopped) {
      this.#onVisibleChange(values);
    }
    return true;
  }

  #withPendingCount(
    state: PreferenceControllerState<TValues>,
  ): PreferenceControllerState<TValues> | SerializedPreferenceControllerState<TValues> {
    return this.#updateMode === 'serialized'
      ? { ...state, pendingCount: this.#pendingCount }
      : state;
  }

  #normalizePatch(patch: Partial<TValues>): Partial<TValues> {
    const normalized: Record<string, unknown> = {};
    for (const key of Object.keys(patch)) {
      if (!Object.hasOwn(this.#definitions, key)) {
        throw new TypeError(`Unknown preference key: ${key}`);
      }
      const typedKey = key as PreferenceKey<TValues>;
      normalized[key] = this.#definitions[typedKey].normalize(patch[typedKey]);
    }
    return normalized as Partial<TValues>;
  }

  #changedKeys(
    previousValues: TValues,
    values: TValues,
    patch: Partial<TValues>,
  ): PreferenceKey<TValues>[] {
    return (Object.keys(patch) as PreferenceKey<TValues>[]).filter(
      (key) => !Object.is(previousValues[key], values[key]),
    );
  }

  #effectContext<TKey extends PreferenceKey<TValues>>(
    key: TKey,
    previousValues: TValues,
    values: TValues,
  ): PreferenceSideEffectContext<TKey, TValues[TKey], TValues> {
    return {
      key,
      previousValue: previousValues[key],
      value: values[key],
      previousValues,
      values,
    };
  }

  async #rollbackPreviews(
    keys: readonly PreferenceKey<TValues>[],
    previousValues: TValues,
    values: TValues,
  ): Promise<Error | null> {
    let firstError: Error | null = null;
    for (const key of [...keys].reverse()) {
      const effect = this.#definitions[key].sideEffect;
      if (effect?.mode !== 'preview-with-rollback') {
        continue;
      }
      try {
        await effect.rollback(this.#effectContext(key, previousValues, values));
      } catch (error) {
        firstError ??= asError(error);
      }
    }
    return firstError;
  }

  async #runPersistedEffects(
    keys: readonly PreferenceKey<TValues>[],
    previousValues: TValues,
    values: TValues,
    revision: number,
  ): Promise<void> {
    for (const key of keys) {
      const effect = this.#definitions[key].sideEffect;
      if (!effect) {
        continue;
      }
      const run = effect.mode === 'after-persistence' ? effect.run : effect.afterPersistence;
      if (!run) {
        continue;
      }
      try {
        await run(this.#effectContext(key, previousValues, values));
      } catch (error) {
        if (effect.onError === 'report') {
          const effectError = asError(error);
          if (revision === this.#identityRevision) {
            this.#setState({ ...this.#state, error: effectError });
          }
          throw effectError;
        }
      }
    }
  }
}

function asError(reason: unknown): Error {
  return reason instanceof Error
    ? reason
    : new Error('Preference operation failed', { cause: reason });
}

export function createPreferenceController<TValues extends object>(
  options: CreatePreferenceControllerOptions<TValues> & { readonly updateMode: 'serialized' },
): SerializedPreferenceController<TValues>;
export function createPreferenceController<TValues extends object>(
  options: CreatePreferenceControllerOptions<TValues>,
): PreferenceController<TValues>;
export function createPreferenceController<TValues extends object>(
  options: CreatePreferenceControllerOptions<TValues>,
): PreferenceController<TValues> {
  return new DefaultPreferenceController(options);
}
