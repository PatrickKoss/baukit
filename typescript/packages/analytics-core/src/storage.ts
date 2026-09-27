import type { AnalyticsStorage } from './types.js';

export interface AnalyticsStorageKeys {
  readonly consent: string;
  readonly anonymousId: string;
  readonly userId: string;
  readonly aliasedUserId: string;
}

/** Keys `AnalyticsClient` reads and writes for a `storageKeyPrefix`. */
export function analyticsStorageKeys(prefix: string): AnalyticsStorageKeys {
  return {
    consent: `${prefix}:consent`,
    anonymousId: `${prefix}:anonymous-id`,
    userId: `${prefix}:user-id`,
    aliasedUserId: `${prefix}:aliased-user-id`,
  };
}

/** Small dependency-free storage useful as the default and in tests. */
export class InMemoryAnalyticsStorage implements AnalyticsStorage {
  readonly #values = new Map<string, string>();

  public constructor(initialValues: Readonly<Record<string, string>> = {}) {
    for (const [key, value] of Object.entries(initialValues)) {
      this.#values.set(key, value);
    }
  }

  public getItem(key: string): string | undefined {
    return this.#values.get(key);
  }

  public setItem(key: string, value: string): void {
    this.#values.set(key, value);
  }

  public removeItem(key: string): void {
    this.#values.delete(key);
  }

  public snapshot(): Readonly<Record<string, string>> {
    return Object.fromEntries(this.#values);
  }
}
