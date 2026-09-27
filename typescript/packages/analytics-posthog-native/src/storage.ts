import type { AnalyticsStorage } from '@baukit/analytics-core';

/** Asynchronous key-value store, such as React Native AsyncStorage. */
export interface AsyncKeyValueStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  removeItem(key: string): Promise<void>;
}

export interface HydratedAnalyticsStorageOptions {
  readonly persistence: AsyncKeyValueStorage;
  /** Keys copied from and written through to `persistence`. Other keys stay in memory. */
  readonly persistentKeys: readonly string[];
}

/**
 * Synchronous `AnalyticsStorage` over an asynchronous store. `load` reads the persistent keys
 * once, then reads are served from memory and writes go through without being awaited.
 */
export class HydratedAnalyticsStorage implements AnalyticsStorage {
  readonly #values: Map<string, string>;
  readonly #persistence: AsyncKeyValueStorage;
  readonly #persistentKeys: ReadonlySet<string>;

  private constructor(
    options: HydratedAnalyticsStorageOptions,
    entries: readonly (readonly [string, string])[],
  ) {
    this.#persistence = options.persistence;
    this.#persistentKeys = new Set(options.persistentKeys);
    this.#values = new Map(entries);
  }

  public static async load(
    options: HydratedAnalyticsStorageOptions,
  ): Promise<HydratedAnalyticsStorage> {
    const keys = [...new Set(options.persistentKeys)];
    const stored = await Promise.all(keys.map((key) => readOrAbsent(options.persistence, key)));
    const entries = keys.flatMap((key, index) => {
      const value = stored[index];
      return value === undefined ? [] : [[key, value] as const];
    });
    return new HydratedAnalyticsStorage(options, entries);
  }

  public getItem(key: string): string | undefined {
    return this.#values.get(key);
  }

  public setItem(key: string, value: string): void {
    this.#values.set(key, value);
    if (!this.#persistentKeys.has(key)) return;
    ignoreFailure(() => this.#persistence.setItem(key, value));
  }

  public removeItem(key: string): void {
    this.#values.delete(key);
    if (!this.#persistentKeys.has(key)) return;
    ignoreFailure(() => this.#persistence.removeItem(key));
  }
}

async function readOrAbsent(
  persistence: AsyncKeyValueStorage,
  key: string,
): Promise<string | undefined> {
  try {
    return (await persistence.getItem(key)) ?? undefined;
  } catch {
    return undefined;
  }
}

function ignoreFailure(write: () => Promise<void>): void {
  try {
    void write().catch(() => undefined);
  } catch {
    return;
  }
}
