import { type JsonValue, type KeyValueStore, normalizeStorageError } from './contracts.js';

/** The part of the Web Storage API (`sessionStorage`, `localStorage`) the adapter uses. */
export interface WebStorageLike {
  readonly length: number;
  key(index: number): string | null;
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** Separates the namespace from the key. Namespaces must not contain it. */
export const WEB_STORAGE_NAMESPACE_DELIMITER = '\u001f';

export interface WebStorageKeyValueStoreOptions {
  /**
   * Moves keys written before the delimiter existed (`namespace + key`) on first use.
   * Only enable it when no other namespace of the origin starts with this one.
   */
  readonly migrateLegacyKeys?: boolean;
}

/**
 * A `KeyValueStore` over Web Storage. Every key is stored as `namespace`, a U+001F
 * delimiter and the key, so `clear` and `clearPrefix` never touch another namespace,
 * even one that starts with this one. Values are stored as JSON text.
 */
export class WebStorageKeyValueStore implements KeyValueStore {
  readonly #storage: WebStorageLike;
  readonly #namespace: string;
  readonly #legacyNamespace: string;
  #migrationPending: boolean;

  public constructor(
    storage: WebStorageLike,
    namespace: string,
    options: WebStorageKeyValueStoreOptions = {},
  ) {
    if (namespace === '') {
      throw new RangeError('Web Storage namespace must not be empty.');
    }
    if (namespace.includes(WEB_STORAGE_NAMESPACE_DELIMITER)) {
      throw new RangeError('Web Storage namespace must not contain U+001F.');
    }
    this.#storage = storage;
    this.#legacyNamespace = namespace;
    this.#namespace = namespace + WEB_STORAGE_NAMESPACE_DELIMITER;
    this.#migrationPending = options.migrateLegacyKeys === true;
  }

  public get(key: string): Promise<JsonValue | undefined> {
    return this.#run(() => {
      const payload = this.#storage.getItem(this.#namespace + key);
      return payload === null ? undefined : parsePayload(payload);
    });
  }

  public set(key: string, value: JsonValue): Promise<void> {
    return this.#run(() => {
      this.#storage.setItem(this.#namespace + key, JSON.stringify(value));
    });
  }

  public delete(key: string): Promise<void> {
    return this.#run(() => {
      this.#storage.removeItem(this.#namespace + key);
    });
  }

  public clear(): Promise<void> {
    return this.clearPrefix('');
  }

  public clearPrefix(prefix: string): Promise<void> {
    return this.#run(() => {
      const storedPrefix = this.#namespace + prefix;
      for (const storedKey of this.#storedKeys()) {
        if (storedKey.startsWith(storedPrefix)) {
          this.#storage.removeItem(storedKey);
        }
      }
    });
  }

  #run<TResult>(operation: () => TResult): Promise<TResult> {
    return run(() => {
      this.#migrateLegacyKeys();
      return operation();
    });
  }

  #migrateLegacyKeys(): void {
    if (!this.#migrationPending) return;
    for (const storedKey of this.#storedKeys()) {
      if (
        !storedKey.startsWith(this.#legacyNamespace) ||
        storedKey.includes(WEB_STORAGE_NAMESPACE_DELIMITER)
      ) {
        continue;
      }
      const payload = this.#storage.getItem(storedKey);
      const key = storedKey.slice(this.#legacyNamespace.length);
      if (payload !== null) this.#storage.setItem(this.#namespace + key, payload);
      this.#storage.removeItem(storedKey);
    }
    this.#migrationPending = false;
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
