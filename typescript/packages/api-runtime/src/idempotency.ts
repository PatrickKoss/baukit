import { ApiError, HttpError, type JsonValue } from './index.js';

/** The server rejected the key because its first request is still running. */
export const IDEMPOTENCY_KEY_IN_PROGRESS_CODE = 'idempotency_key_in_progress' as const;

/**
 * What the client knows about one mutation attempt.
 *
 * `possibly-committed` means the server may have applied the effect, so a retry must reuse the key.
 */
export type MutationAttemptOutcome = 'committed' | 'not-committed' | 'possibly-committed';

/** One logical write. Two intents with equal fields share a key. */
export interface MutationIntent {
  /** The signed-in account, so keys never cross an account switch. */
  readonly account: string;
  /** A stable operation name, such as `createNote`. Include the target ID when there is one. */
  readonly operation: string;
  /** The request body, plus the expected revision for conditional writes. */
  readonly body: JsonValue;
}

/** A key kept for one intent. */
export interface StoredIdempotencyKey {
  readonly key: string;
  readonly createdAtMs: number;
}

/**
 * Where an idempotency key store keeps keys.
 *
 * `slot` is the canonical JSON of the intent. A persistent adapter should store a digest of it,
 * not the text, when request bodies are sensitive.
 */
export interface IdempotencyKeyStorage {
  get(slot: string): StoredIdempotencyKey | null | Promise<StoredIdempotencyKey | null>;
  set(slot: string, value: StoredIdempotencyKey): void | Promise<void>;
  delete(slot: string): void | Promise<void>;
}

/** Options for {@link createIdempotencyKeyStore}. */
export interface IdempotencyKeyStoreOptions {
  /** How long a key is reused. Keep it below the server's replay retention. */
  readonly ttlMs: number;
  /** Clock seam. Defaults to `Date.now`. */
  readonly now?: () => number;
  /** Key factory. Defaults to `crypto.randomUUID`. */
  readonly keyFactory?: () => string;
  /** Key storage. Defaults to an in-memory map that a reload clears. */
  readonly storage?: IdempotencyKeyStorage;
}

/** Keeps one key per account, operation, and body until a definite outcome or expiry. */
export interface IdempotencyKeyStore {
  /** Returns the live key for the intent, or creates one. */
  keyFor(intent: MutationIntent): Promise<string>;
  /** Drops the key after a definite outcome and keeps it while the outcome is uncertain. */
  settle(intent: MutationIntent, outcome: MutationAttemptOutcome): Promise<void>;
}

const OK_MIN = 200;
const OK_MAX = 299;
const CLIENT_ERROR_MIN = 400;
const SERVER_ERROR_MIN = 500;
const REQUEST_TIMEOUT = 408;
const TOO_MANY_REQUESTS = 429;

/** Classifies a mutation response status. */
export function classifyMutationStatus(status: number): MutationAttemptOutcome {
  if (status >= OK_MIN && status <= OK_MAX) {
    return 'committed';
  }
  if (status === REQUEST_TIMEOUT || status === TOO_MANY_REQUESTS || status >= SERVER_ERROR_MIN) {
    return 'possibly-committed';
  }
  if (status >= CLIENT_ERROR_MIN) {
    return 'not-committed';
  }
  return 'possibly-committed';
}

/**
 * Classifies an error thrown by an `@baukit/api-runtime` fetch.
 *
 * Network errors, aborts, and unknown throws are `possibly-committed`, because the request may
 * have reached the server. So is 409 `idempotency_key_in_progress`.
 */
export function classifyMutationError(error: unknown): MutationAttemptOutcome {
  if (error instanceof ApiError) {
    if (error.code === IDEMPOTENCY_KEY_IN_PROGRESS_CODE) {
      return 'possibly-committed';
    }
    return classifyMutationStatus(error.status);
  }
  if (error instanceof HttpError) {
    return classifyMutationStatus(error.status);
  }
  return 'possibly-committed';
}

/** Creates a key store. */
export function createIdempotencyKeyStore(
  options: IdempotencyKeyStoreOptions,
): IdempotencyKeyStore {
  if (!Number.isFinite(options.ttlMs) || options.ttlMs <= 0) {
    throw new Error('ttlMs must be a finite positive number');
  }
  const now = options.now ?? Date.now;
  const keyFactory = options.keyFactory ?? (() => globalThis.crypto.randomUUID());
  const storage = options.storage ?? memoryStorage();

  return {
    async keyFor(intent) {
      const slot = intentSlot(intent);
      const stored = await storage.get(slot);
      const current = now();
      if (stored !== null && current - stored.createdAtMs < options.ttlMs) {
        return stored.key;
      }
      const fresh = { key: keyFactory(), createdAtMs: current };
      await storage.set(slot, fresh);
      return fresh.key;
    },
    async settle(intent, outcome) {
      if (outcome === 'possibly-committed') {
        return;
      }
      await storage.delete(intentSlot(intent));
    },
  };
}

/**
 * Sends one keyed mutation and settles its key.
 *
 * `send` receives the key to put in the `Idempotency-Key` header. A throw is classified with
 * {@link classifyMutationError}, settled, and rethrown.
 */
export async function sendIdempotentMutation<Result>(
  store: IdempotencyKeyStore,
  intent: MutationIntent,
  send: (key: string) => Promise<Result>,
): Promise<Result> {
  const key = await store.keyFor(intent);
  let result: Result;
  try {
    result = await send(key);
  } catch (error) {
    await store.settle(intent, classifyMutationError(error));
    throw error;
  }
  await store.settle(intent, 'committed');
  return result;
}

/** Serializes JSON with object members sorted by key, so equal values give equal text. */
export function canonicalJson(value: JsonValue): string {
  if (isJsonArray(value)) {
    return `[${value.map((item) => canonicalJson(item)).join(',')}]`;
  }
  if (value !== null && typeof value === 'object') {
    const members = Object.entries(value)
      .sort(([left], [right]) => (left < right ? -1 : 1))
      .map(([key, member]) => `${JSON.stringify(key)}:${canonicalJson(member)}`);
    return `{${members.join(',')}}`;
  }
  if (typeof value === 'number' && !Number.isFinite(value)) {
    throw new Error('canonical JSON cannot encode a non-finite number');
  }
  return JSON.stringify(value);
}

function isJsonArray(value: JsonValue): value is readonly JsonValue[] {
  return Array.isArray(value);
}

function intentSlot(intent: MutationIntent): string {
  return canonicalJson([intent.account, intent.operation, intent.body]);
}

function memoryStorage(): IdempotencyKeyStorage {
  const keys = new Map<string, StoredIdempotencyKey>();
  return {
    get: (slot) => keys.get(slot) ?? null,
    set: (slot, value) => {
      keys.set(slot, value);
    },
    delete: (slot) => {
      keys.delete(slot);
    },
  };
}
