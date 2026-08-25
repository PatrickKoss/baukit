import AsyncStorage from '@react-native-async-storage/async-storage';
import * as Crypto from 'expo-crypto';
import type { AnalyticsClient, AnalyticsStorage } from '@baukit/analytics-core';

import { analyticsStoragePrefix, createAnalytics, type ProductEvent } from './analytics-client';

const persistentStorageKeys = [
  `${analyticsStoragePrefix}:anonymous-id`,
  `${analyticsStoragePrefix}:user-id`,
  `${analyticsStoragePrefix}:aliased-user-id`,
] as const;
const persistentStorageKeySet = new Set<string>(persistentStorageKeys);

class HydratedAnalyticsStorage implements AnalyticsStorage {
  readonly #values: Map<string, string>;

  private constructor(entries: readonly (readonly [string, string])[]) {
    this.#values = new Map(entries);
  }

  static async load(): Promise<HydratedAnalyticsStorage> {
    try {
      const stored = await AsyncStorage.multiGet(persistentStorageKeys);
      const entries: [string, string][] = [];
      for (const [key, value] of stored) {
        if (value !== null) {
          entries.push([key, value]);
        }
      }
      return new HydratedAnalyticsStorage(entries);
    } catch {
      return new HydratedAnalyticsStorage([]);
    }
  }

  getItem(key: string): string | undefined {
    return this.#values.get(key);
  }

  setItem(key: string, value: string): void {
    this.#values.set(key, value);
    if (persistentStorageKeySet.has(key)) {
      void AsyncStorage.setItem(key, value).catch(() => undefined);
    }
  }

  removeItem(key: string): void {
    this.#values.delete(key);
    if (persistentStorageKeySet.has(key)) {
      void AsyncStorage.removeItem(key).catch(() => undefined);
    }
  }
}

let analyticsPromise: Promise<AnalyticsClient<ProductEvent>> | undefined;

export function loadAnalytics(): Promise<AnalyticsClient<ProductEvent>> {
  analyticsPromise ??= HydratedAnalyticsStorage.load().then((storage) =>
    createAnalytics(() => Crypto.randomUUID(), __DEV__ ? 'development' : 'production', storage),
  );
  return analyticsPromise;
}
