import { ApiError, normalizeResponseError, type FetchImplementation } from './index.js';
import {
  classifyMutationError,
  createIdempotencyKeyStore,
  type IdempotencyKeyStorage,
} from './idempotency.js';

export type ProfileErasureReceipt =
  | { readonly status: 'completed'; readonly operationId: string; readonly completedAt: string }
  | { readonly status: 'pending'; readonly operationId: string | null };

export type ProfileErasureOperation =
  | { readonly status: 'pending' | 'failed'; readonly operationId: string }
  | { readonly status: 'completed'; readonly operationId: string; readonly completedAt?: string };

export interface ProfileErasureClientOptions {
  readonly fetch: FetchImplementation;
  readonly account: string;
  /** Durable storage, separate from tokens so a lost response survives a reload. */
  readonly storage: IdempotencyKeyStorage;
  readonly keyFactory?: () => string;
}

export interface ProfileErasurePollingOptions {
  readonly signal?: AbortSignal;
  /** Total status reads. Defaults to 8. A still-pending operation is returned at the limit. */
  readonly maxAttempts?: number;
  readonly initialDelayMs?: number;
  readonly maxDelayMs?: number;
}

export interface ProfileErasureClient {
  erase(signal?: AbortSignal): Promise<ProfileErasureReceipt>;
  poll(
    operationId: string,
    options?: ProfileErasurePollingOptions,
  ): Promise<ProfileErasureOperation>;
}

/** A committed receipt whose durable key could not be removed from this device. */
export class ProfileErasureKeyCleanupError extends Error {
  public override readonly name = 'ProfileErasureKeyCleanupError';

  public constructor(
    public readonly receipt: ProfileErasureReceipt,
    cause: unknown,
  ) {
    super('The committed erasure key could not be removed.', { cause });
  }
}

/** Compatible with the ambiguous error contract in @baukit/data-contracts. */
export class AmbiguousProfileErasureError extends Error {
  public override readonly name = 'AmbiguousProfileErasureError';
  public readonly code = 'product_profile_erasure_ambiguous' as const;

  public constructor(cause: unknown) {
    super('The server erasure outcome is unknown.', { cause });
  }
}

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u;
const IDEMPOTENCY_KEY_PATTERN = /^[\x21-\x7e]{16,128}$/u;
const INITIAL_DELAY_MS = 500;
const MAX_DELAY_MS = 5_000;
const MAX_ATTEMPTS = 8;

/** Creates a client for DELETE /me and its fenced-subject status endpoint. */
export function createProfileErasureClient(
  options: ProfileErasureClientOptions,
): ProfileErasureClient {
  if (options.account.trim() === '') throw new TypeError('An erasure account is required.');
  const keys = createIdempotencyKeyStore({
    // An uncertain erasure must keep its key, even if the user returns much later.
    ttlMs: Number.MAX_SAFE_INTEGER,
    storage: options.storage,
    ...(options.keyFactory === undefined ? {} : { keyFactory: options.keyFactory }),
  });
  const intent = { account: options.account, operation: 'profile.erase', body: null };

  return {
    async erase(signal) {
      signal?.throwIfAborted();
      const key = await keys.keyFor(intent);
      if (!IDEMPOTENCY_KEY_PATTERN.test(key))
        throw new TypeError('Invalid erasure idempotency key.');
      signal?.throwIfAborted();
      let receipt: ProfileErasureReceipt;
      try {
        const response = await options.fetch('/me', {
          method: 'DELETE',
          headers: { 'Idempotency-Key': key },
          ...(signal === undefined ? {} : { signal }),
        });
        if (!response.ok) throw await normalizeResponseError(response);
        const value: unknown = await response.json();
        receipt = parseReceipt(value, response.status);
      } catch (cause) {
        if (isErasedFence(cause)) {
          receipt = { status: 'pending', operationId: null };
        } else if (classifyMutationError(cause) === 'not-committed') {
          if (cause instanceof ApiError && cause.code === 'erasure_idempotency_conflict') {
            await keys.settle(intent, 'not-committed');
          }
          throw cause;
        } else throw new AmbiguousProfileErasureError(cause);
      }
      try {
        await keys.settle(intent, 'committed');
      } catch (cause) {
        throw new ProfileErasureKeyCleanupError(receipt, cause);
      }
      return receipt;
    },
    async poll(operationId, polling = {}) {
      if (!UUID_PATTERN.test(operationId)) throw new TypeError('Invalid erasure operation ID.');
      const maxAttempts = positiveInteger(polling.maxAttempts ?? MAX_ATTEMPTS, 'maxAttempts');
      let delayMs = positiveInteger(polling.initialDelayMs ?? INITIAL_DELAY_MS, 'initialDelayMs');
      const maxDelayMs = positiveInteger(polling.maxDelayMs ?? MAX_DELAY_MS, 'maxDelayMs');
      delayMs = Math.min(delayMs, maxDelayMs);
      for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
        polling.signal?.throwIfAborted();
        const operation = await readOperation(options.fetch, operationId, polling.signal);
        if (operation.status !== 'pending' || attempt === maxAttempts - 1) return operation;
        await waitForPoll(delayMs, polling.signal);
        delayMs = Math.min(delayMs * 2, maxDelayMs);
      }
      throw new RangeError('Invalid erasure polling limit.');
    },
  };
}

function isErasedFence(cause: unknown): boolean {
  return cause instanceof ApiError && cause.status === 401 && cause.code === 'profile_erased';
}

async function readOperation(
  fetch: FetchImplementation,
  operationId: string,
  signal: AbortSignal | undefined,
): Promise<ProfileErasureOperation> {
  try {
    const response = await fetch(`/me/erasures/${encodeURIComponent(operationId)}`, {
      method: 'GET',
      ...(signal === undefined ? {} : { signal }),
    });
    signal?.throwIfAborted();
    if (!response.ok) throw await normalizeResponseError(response);
    const value: unknown = await response.json();
    const operation = parseOperation(value);
    if (operation.operationId !== operationId)
      throw new TypeError('Erasure operation ID mismatch.');
    return operation;
  } catch (cause) {
    if (isErasedFence(cause)) return { status: 'pending', operationId };
    throw cause;
  }
}

function parseReceipt(value: unknown, status: number): ProfileErasureReceipt {
  const operation = parseOperation(value);
  if (status === 202 && operation.status === 'pending') {
    return { status: 'pending', operationId: operation.operationId };
  }
  if (status === 200 && operation.status === 'completed' && operation.completedAt !== undefined) {
    return { ...operation, completedAt: operation.completedAt };
  }
  throw new TypeError('Invalid erasure receipt.');
}

function parseOperation(value: unknown): ProfileErasureOperation {
  if (typeof value !== 'object' || value === null)
    throw new TypeError('Invalid erasure operation.');
  const operationId: unknown = Reflect.get(value, 'operationId');
  const status: unknown = Reflect.get(value, 'status');
  const completedAt: unknown = Reflect.get(value, 'completedAt');
  if (typeof operationId !== 'string' || !UUID_PATTERN.test(operationId)) {
    throw new TypeError('Invalid erasure operation ID.');
  }
  if (status === 'pending' || status === 'failed') return { operationId, status };
  if (status !== 'completed') throw new TypeError('Invalid erasure operation status.');
  if (completedAt === undefined) return { operationId, status };
  if (
    typeof completedAt !== 'string' ||
    !TIMESTAMP_PATTERN.test(completedAt) ||
    !Number.isFinite(Date.parse(completedAt))
  ) {
    throw new TypeError('Invalid erasure completion time.');
  }
  return { operationId, status, completedAt };
}

function positiveInteger(value: number, name: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) throw new RangeError(`${name} must be positive.`);
  return value;
}

function waitForPoll(delayMs: number, signal: AbortSignal | undefined): Promise<void> {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      globalThis.clearTimeout(timer);
      const reason: unknown = signal?.reason;
      reject(
        reason instanceof Error ? reason : new Error('Erasure polling aborted.', { cause: reason }),
      );
    };
    const timer = globalThis.setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, delayMs);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}
