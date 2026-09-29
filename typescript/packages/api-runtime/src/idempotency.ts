import { ApiError, HttpError, type JsonValue } from './index.js';

/** The server rejected the key because its first request is still running. */
export const IDEMPOTENCY_KEY_IN_PROGRESS_CODE = 'idempotency_key_in_progress' as const;

/**
 * What the client knows about one mutation attempt.
 *
 * `possibly-committed` means the server may have applied the effect, so a retry must reuse the key.
 */
export type MutationAttemptOutcome = 'committed' | 'not-committed' | 'possibly-committed';

/**
 * Accepts `Value` when it serializes to JSON unchanged, including interfaces that have no index
 * signature. Object members may be `undefined`, which JSON omits.
 */
export type JsonCompatible<Value> = { readonly [Key in keyof Value]: JsonMember<Value[Key]> };

type JsonMember<Value> = Value extends undefined | JsonValue
  ? Value
  : Value extends (...args: never[]) => unknown
    ? never
    : JsonCompatible<Value>;

/** One logical write. Two intents with equal fields share a key. */
export interface MutationIntent<Body extends JsonCompatible<Body> = JsonValue> {
  /** The signed-in account, so keys never cross an account switch. */
  readonly account: string;
  /** A stable operation name, such as `createNote`. Include the target ID when there is one. */
  readonly operation: string;
  /** The request body, plus the expected revision for conditional writes. */
  readonly body: Body;
}

/** Classifies a product error, or returns `undefined` to fall back to {@link classifyMutationError}. */
export type MutationErrorClassifier = (error: unknown) => MutationAttemptOutcome | undefined;

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
  /** Classifies the product's own API errors before the Baukit defaults apply. */
  readonly classifyError?: MutationErrorClassifier;
}

/** Keeps one key per account, operation, and body until a definite outcome or expiry. */
export interface IdempotencyKeyStore {
  /**
   * Returns the live key for the intent, or creates one. Calls for the same intent share one key
   * from the first call until the intent settles, even when storage is slow or drops writes.
   */
  keyFor<Body extends JsonCompatible<Body>>(intent: MutationIntent<Body>): Promise<string>;
  /** Drops the key after a definite outcome and keeps it while the outcome is uncertain. */
  settle<Body extends JsonCompatible<Body>>(
    intent: MutationIntent<Body>,
    outcome: MutationAttemptOutcome,
  ): Promise<void>;
  /** Classifies a failed send with the product classifier, then {@link classifyMutationError}. */
  classifyError(error: unknown): MutationAttemptOutcome;
}

const OK_MIN = 200;
const OK_MAX = 299;
const CLIENT_ERROR_MIN = 400;
const SERVER_ERROR_MIN = 500;
const REQUEST_TIMEOUT = 408;
const TOO_MANY_REQUESTS = 429;

/**
 * Classifies a mutation response status and, when the caller has it, the error code. The code
 * `idempotency_key_in_progress` is `possibly-committed` whatever the status.
 */
export function classifyMutationStatus(status: number, code?: string): MutationAttemptOutcome {
  if (code === IDEMPOTENCY_KEY_IN_PROGRESS_CODE) {
    return 'possibly-committed';
  }
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
  const classifyProductError = options.classifyError ?? (() => undefined);
  const unsettled = new Map<string, Promise<StoredIdempotencyKey>>();
  const isLive = (stored: StoredIdempotencyKey) => now() - stored.createdAtMs < options.ttlMs;

  const loadOrCreate = async (slot: string): Promise<StoredIdempotencyKey> => {
    const stored = await storage.get(slot);
    if (stored !== null && isLive(stored)) {
      return stored;
    }
    const fresh = { key: keyFactory(), createdAtMs: now() };
    await storage.set(slot, fresh);
    return fresh;
  };

  const reuseOrLoad = async (
    slot: string,
    earlier: Promise<StoredIdempotencyKey> | undefined,
  ): Promise<StoredIdempotencyKey> => {
    const held = await earlier?.catch(() => undefined);
    if (held !== undefined && isLive(held)) {
      return held;
    }
    return loadOrCreate(slot);
  };

  return {
    async keyFor(intent) {
      const slot = intentSlot(intent);
      const pending = reuseOrLoad(slot, unsettled.get(slot));
      unsettled.set(slot, pending);
      return (await pending).key;
    },
    async settle(intent, outcome) {
      const slot = intentSlot(intent);
      unsettled.delete(slot);
      if (outcome === 'possibly-committed') {
        return;
      }
      await storage.delete(slot);
    },
    classifyError(error) {
      return classifyProductError(error) ?? classifyMutationError(error);
    },
  };
}

/**
 * Sends one keyed mutation and settles its key.
 *
 * `send` receives the key to put in the `Idempotency-Key` header. A throw is classified with the
 * store's `classifyError`, settled, and rethrown.
 */
export async function sendIdempotentMutation<Result, Body extends JsonCompatible<Body> = JsonValue>(
  store: IdempotencyKeyStore,
  intent: MutationIntent<Body>,
  send: (key: string) => Promise<Result>,
): Promise<Result> {
  const key = await store.keyFor(intent);
  let result: Result;
  try {
    result = await send(key);
  } catch (error) {
    await store.settle(intent, store.classifyError(error));
    throw error;
  }
  await store.settle(intent, 'committed');
  return result;
}

/**
 * Serializes JSON with object members sorted by key, so equal values give equal text. Members
 * whose value is `undefined` are left out, as `JSON.stringify` leaves them out.
 */
export function canonicalJson<Value extends JsonCompatible<Value>>(value: Value): string {
  return canonicalText(value);
}

function canonicalText(value: unknown): string {
  if (value === undefined) {
    throw new Error('canonical JSON cannot encode undefined');
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalItem).join(',')}]`;
  }
  if (value !== null && typeof value === 'object') {
    const members = Object.entries(value)
      .filter(([, member]) => member !== undefined)
      .sort(([left], [right]) => (left < right ? -1 : 1))
      .map(([key, member]) => `${JSON.stringify(key)}:${canonicalText(member)}`);
    return `{${members.join(',')}}`;
  }
  if (typeof value === 'number' && !Number.isFinite(value)) {
    throw new Error('canonical JSON cannot encode a non-finite number');
  }
  return JSON.stringify(value);
}

function canonicalItem(item: unknown): string {
  return item === undefined ? 'null' : canonicalText(item);
}

function intentSlot<Body extends JsonCompatible<Body>>(intent: MutationIntent<Body>): string {
  return canonicalText([intent.account, intent.operation, intent.body]);
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
