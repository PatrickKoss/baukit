import { AnalyticsClient, analyticsStorageKeys } from '@baukit/analytics-core';
import { describe, expect, it, vi } from 'vitest';

import { HydratedAnalyticsStorage, type AsyncKeyValueStorage } from './storage.js';

const keys = analyticsStorageKeys('app');
const persistentKeys = [keys.anonymousId, keys.userId, keys.aliasedUserId];
const ANONYMOUS_ID = '00000000-0000-4000-8000-000000000001';

class MemoryPersistence implements AsyncKeyValueStorage {
  readonly values = new Map<string, string>();
  readonly failingReads = new Set<string>();
  failWrites = false;

  getItem(key: string): Promise<string | null> {
    if (this.failingReads.has(key)) return Promise.reject(new Error('read failed'));
    return Promise.resolve(this.values.get(key) ?? null);
  }

  setItem(key: string, value: string): Promise<void> {
    if (this.failWrites) return Promise.reject(new Error('write failed'));
    this.values.set(key, value);
    return Promise.resolve();
  }

  removeItem(key: string): Promise<void> {
    if (this.failWrites) return Promise.reject(new Error('write failed'));
    this.values.delete(key);
    return Promise.resolve();
  }
}

describe('HydratedAnalyticsStorage', () => {
  it('hydrates only the persistent keys that hold a value', async () => {
    const persistence = new MemoryPersistence();
    persistence.values.set(keys.anonymousId, ANONYMOUS_ID);
    persistence.values.set(keys.consent, 'granted');

    const storage = await HydratedAnalyticsStorage.load({ persistence, persistentKeys });

    expect(storage.getItem(keys.anonymousId)).toBe(ANONYMOUS_ID);
    expect(storage.getItem(keys.userId)).toBeUndefined();
    expect(storage.getItem(keys.consent)).toBeUndefined();
  });

  it('keeps the keys that load when another key fails to read', async () => {
    const persistence = new MemoryPersistence();
    persistence.values.set(keys.anonymousId, ANONYMOUS_ID);
    persistence.values.set(keys.userId, 'user');
    persistence.failingReads.add(keys.userId);

    const storage = await HydratedAnalyticsStorage.load({ persistence, persistentKeys });

    expect(storage.getItem(keys.anonymousId)).toBe(ANONYMOUS_ID);
    expect(storage.getItem(keys.userId)).toBeUndefined();
  });

  it('writes persistent keys through and keeps other keys in memory', async () => {
    const persistence = new MemoryPersistence();
    const storage = await HydratedAnalyticsStorage.load({ persistence, persistentKeys });

    storage.setItem(keys.userId, 'user');
    storage.setItem(keys.consent, 'granted');
    await Promise.resolve();

    expect(storage.getItem(keys.consent)).toBe('granted');
    expect(persistence.values.get(keys.userId)).toBe('user');
    expect(persistence.values.has(keys.consent)).toBe(false);

    storage.removeItem(keys.userId);
    await Promise.resolve();
    expect(storage.getItem(keys.userId)).toBeUndefined();
    expect(persistence.values.has(keys.userId)).toBe(false);
  });

  it('keeps the in-memory value when a write fails or throws', async () => {
    const persistence = new MemoryPersistence();
    const storage = await HydratedAnalyticsStorage.load({ persistence, persistentKeys });
    persistence.failWrites = true;

    storage.setItem(keys.userId, 'user');
    expect(storage.getItem(keys.userId)).toBe('user');

    vi.spyOn(persistence, 'removeItem').mockImplementation(() => {
      throw new Error('synchronous failure');
    });
    expect(() => {
      storage.removeItem(keys.userId);
    }).not.toThrow();
    expect(storage.getItem(keys.userId)).toBeUndefined();
  });

  it('restores the anonymous identity for a new analytics client', async () => {
    const persistence = new MemoryPersistence();
    const first = new AnalyticsClient({
      context: {
        schema_version: 1,
        app: 'example-native',
        app_version: '1.0.0',
        platform: 'native',
        environment: 'test',
        locale: 'en-GB',
      },
      allowlist: {},
      storageKeyPrefix: 'app',
      uuidFactory: () => ANONYMOUS_ID,
      storage: await HydratedAnalyticsStorage.load({ persistence, persistentKeys }),
    });
    await Promise.resolve();

    const restored = await HydratedAnalyticsStorage.load({ persistence, persistentKeys });
    expect(restored.getItem(keys.anonymousId)).toBe(first.anonymousId);
  });
});
