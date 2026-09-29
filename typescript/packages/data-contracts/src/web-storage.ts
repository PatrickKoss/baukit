import { type JsonValue, type KeyValueStore, normalizeStorageError } from './contracts.js';

/** The part of the Web Storage API (`sessionStorage`, `localStorage`) the adapter uses. */
export interface WebStorageLike {
  readonly length: number;
  key(index: number): string | null;
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/**
 * A `KeyValueStore` over Web Storage. Every key is stored under `namespace`, so `clear` and
 * `clearPrefix` never touch other data of the origin. Values are stored as JSON text.
 */
export class WebStorageKeyValueStore implements KeyValueStore {
  readonly #storage: WebStorageLike;
  readonly #namespace: string;

  public constructor(storage: WebStorageLike, namespace: string) {
    if (namespace === '') {
      throw new RangeError('Web Storage namespace must not be empty.');
    }
    this.#storage = storage;
    this.#namespace = namespace;
  }

  public get(key: string): Promise<JsonValue | undefined> {
    return run(() => {
      const payload = this.#storage.getItem(this.#namespace + key);
      return payload === null ? undefined : parsePayload(payload);
    });
  }

  public set(key: string, value: JsonValue): Promise<void> {
    return run(() => {
      this.#storage.setItem(this.#namespace + key, JSON.stringify(value));
    });
  }

  public delete(key: string): Promise<void> {
    return run(() => {
      this.#storage.removeItem(this.#namespace + key);
    });
  }

  public clear(): Promise<void> {
    return this.clearPrefix('');
  }

  public clearPrefix(prefix: string): Promise<void> {
    return run(() => {
      const storedPrefix = this.#namespace + prefix;
      for (const storedKey of this.#storedKeys()) {
        if (storedKey.startsWith(storedPrefix)) {
          this.#storage.removeItem(storedKey);
        }
      }
    });
  }

  #storedKeys(): string[] {
    const keys: string[] = [];
    for (let index = 0; index < this.#storage.length; index += 1) {
      const key = this.#storage.key(index);
      if (key !== null) {
        keys.push(key);
      }
    }
    return keys;
  }
}

function run<TResult>(operation: () => TResult): Promise<TResult> {
  return Promise.resolve()
    .then(operation)
    .catch((cause: unknown) => {
      throw normalizeStorageError(cause);
    });
}

function parsePayload(payload: string): JsonValue {
  try {
    return JSON.parse(payload) as JsonValue;
  } catch {
    throw new TypeError('Web Storage contains an invalid key/value payload.');
  }
}
