import { describe, expect, it } from 'vitest';

import { describeKeyValueContract } from './vitest.js';
import { type WebStorageLike, WebStorageKeyValueStore } from './web-storage.js';

class MapStorage implements WebStorageLike {
  readonly items = new Map<string, string>();
  failure: Error | null = null;

  get length(): number {
    return this.items.size;
  }

  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null;
  }

  getItem(key: string): string | null {
    this.#throwIfFailing();
    return this.items.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.#throwIfFailing();
    this.items.set(key, value);
  }

  removeItem(key: string): void {
    this.#throwIfFailing();
    this.items.delete(key);
  }

  #throwIfFailing(): void {
    if (this.failure !== null) {
      throw this.failure;
    }
  }
}

function namedError(name: string): Error {
  const error = new Error('provider text');
  error.name = name;
  return error;
}

describeKeyValueContract(() => new WebStorageKeyValueStore(new MapStorage(), 'app:'));

describe('WebStorageKeyValueStore', () => {
  it('rejects an empty namespace', () => {
    expect(() => new WebStorageKeyValueStore(new MapStorage(), '')).toThrow(RangeError);
  });

  it('stores JSON under the namespace and leaves other keys of the origin alone', async () => {
    const storage = new MapStorage();
    storage.items.set('foreign', 'kept');
    storage.items.set('app-other:a', 'kept');
    const store = new WebStorageKeyValueStore(storage, 'app:');

    await store.set('a', { ready: true });
    expect(storage.items.get('app:a')).toBe('{"ready":true}');

    await store.clear();
    expect([...storage.items.keys()]).toEqual(['foreign', 'app-other:a']);
  });

  it('reports quota failures as StorageError', async () => {
    const storage = new MapStorage();
    storage.failure = namedError('QuotaExceededError');
    const store = new WebStorageKeyValueStore(storage, 'app:');

    await expect(store.set('a', 1)).rejects.toMatchObject({
      name: 'StorageError',
      code: 'storage_quota_exceeded',
    });
  });

  it('rejects instead of throwing when storage access is denied', async () => {
    const storage = new MapStorage();
    storage.failure = namedError('SecurityError');
    const store = new WebStorageKeyValueStore(storage, 'app:');

    const read = store.get('a');
    await expect(read).rejects.toMatchObject({ name: 'SecurityError' });
    await expect(store.set('a', 1)).rejects.toMatchObject({ name: 'SecurityError' });
  });

  it('rejects a payload that is not JSON without echoing it', async () => {
    const storage = new MapStorage();
    storage.items.set('app:a', 'secret{');
    const store = new WebStorageKeyValueStore(storage, 'app:');

    await expect(store.get('a')).rejects.toThrow(TypeError);
    await expect(store.get('a')).rejects.not.toThrow(/secret/);
  });
});
