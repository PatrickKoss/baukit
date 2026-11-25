import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { type WebStorageLike, WebStorageKeyValueStore } from './web-storage.js';

class MapStorage implements WebStorageLike {
  public readonly items = new Map<string, string>();
  public get length(): number {
    return this.items.size;
  }
  public key(index: number): string | null {
    return [...this.items.keys()][index] ?? null;
  }
  public getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  public setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
  public removeItem(key: string): void {
    this.items.delete(key);
  }
}

const namespace = fc.string({ minLength: 1, maxLength: 4 }).filter((value) => !value.includes('\u001f'));
const key = fc.string({ maxLength: 4 });
const operation = fc.oneof(
  fc.tuple(fc.constant('set' as const), key, fc.integer()),
  fc.tuple(fc.constant('delete' as const), key, fc.constant(0)),
  fc.tuple(fc.constant('clear' as const), fc.constant(''), fc.constant(0)),
  fc.tuple(fc.constant('clearPrefix' as const), key, fc.constant(0)),
);

async function snapshot(store: WebStorageKeyValueStore, keys: readonly string[]) {
  return Promise.all(keys.map((name) => store.get(name)));
}

describe('C-04 web storage namespace isolation', () => {
  it('operations on n1 never change what n2 sees', async () => {
    await fc.assert(
      fc.asyncProperty(
        namespace,
        namespace,
        fc.array(fc.tuple(key, fc.integer()), { maxLength: 6 }),
        fc.array(operation, { maxLength: 10 }),
        async (n1, n2, seed, operations) => {
          fc.pre(n1 !== n2);
          const storage = new MapStorage();
          const first = new WebStorageKeyValueStore(storage, n1);
          const second = new WebStorageKeyValueStore(storage, n2);
          for (const [name, value] of seed) await second.set(name, value);
          const keys = [...new Set([...seed.map(([name]) => name), ...operations.map(([, name]) => name)])];
          const before = await snapshot(second, keys);
          for (const [kind, name, value] of operations) {
            if (kind === 'set') await first.set(name, value);
            else if (kind === 'delete') await first.delete(name);
            else if (kind === 'clear') await first.clear();
            else await first.clearPrefix(name);
          }
          expect(await snapshot(second, keys)).toEqual(before);
        },
      ),
    );
  });
});
