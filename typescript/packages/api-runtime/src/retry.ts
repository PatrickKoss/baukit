import { fullJitterBackoffMs } from './backoff.js';
import { NetworkError } from './errors.js';
import { IDEMPOTENCY_KEY_HEADER } from './headers.js';
import type { RetryableMethod, RetryOptions } from './index.js';

export interface NormalizedRetryOptions {
  readonly maxRetries: number;
  readonly baseDelayMs: number;
  readonly maxDelayMs: number;
  readonly methods: ReadonlySet<RetryableMethod>;
  readonly random: () => number;
  readonly sleep: (delayMs: number, signal: AbortSignal | null) => Promise<void>;
}

const KEYED_RETRY_ONLY = new Set(['POST', 'PATCH']);
const RETRYABLE_METHODS = new Set<string>([
  'GET',
  'HEAD',
  'OPTIONS',
  'PUT',
  'DELETE',
  'POST',
  'PATCH',
]);
const RETRYABLE_STATUSES = new Set([502, 503, 504]);

const DEFAULT_MAX_RETRIES = 2;
const DEFAULT_BASE_DELAY_MS = 100;
const DEFAULT_MAX_DELAY_MS = 2_000;
const DISABLED_RETRY: RetryOptions = { maxRetries: 0, methods: [] };

export function normalizeRetryOptions(
  options: RetryOptions | false | undefined,
): NormalizedRetryOptions {
  const configured = options === false ? DISABLED_RETRY : (options ?? {});
  const maxRetries = configured.maxRetries ?? DEFAULT_MAX_RETRIES;
  const baseDelayMs = configured.baseDelayMs ?? DEFAULT_BASE_DELAY_MS;
  const maxDelayMs = configured.maxDelayMs ?? DEFAULT_MAX_DELAY_MS;
  assertFiniteNonNegativeInteger(maxRetries, 'maxRetries');
  assertFiniteNonNegative(baseDelayMs, 'baseDelayMs');
  assertFiniteNonNegative(maxDelayMs, 'maxDelayMs');
  const requestedMethods = configured.methods ?? ['GET', 'HEAD'];
  const methods = new Set(
    requestedMethods.filter((method): method is RetryableMethod => RETRYABLE_METHODS.has(method)),
  );
  return {
    maxRetries,
    baseDelayMs,
    maxDelayMs,
    methods,
    random: configured.random ?? Math.random,
    sleep: configured.sleep ?? abortableSleep,
  };
}

function assertFiniteNonNegative(value: number, name: string): void {
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`${name} must be a finite non-negative number`);
  }
}

function assertFiniteNonNegativeInteger(value: number, name: string): void {
  assertFiniteNonNegative(value, name);
  if (!Number.isInteger(value)) {
    throw new Error(`${name} must be an integer`);
  }
}

export function shouldRetryMethod(request: Request, retry: NormalizedRetryOptions): boolean {
  const method = request.method;
  if (!retry.methods.has(method as RetryableMethod)) {
    return false;
  }
  return !KEYED_RETRY_ONLY.has(method) || request.headers.has(IDEMPOTENCY_KEY_HEADER);
}

export function shouldRetryResponse(
  request: Request,
  status: number,
  retries: number,
  retry: NormalizedRetryOptions,
): boolean {
  return (
    retries < retry.maxRetries &&
    shouldRetryMethod(request, retry) &&
    RETRYABLE_STATUSES.has(status)
  );
}

export async function retryDelay(
  retryIndex: number,
  retry: NormalizedRetryOptions,
  signal: AbortSignal,
  requestId: string,
): Promise<void> {
  try {
    await retry.sleep(fullJitterBackoffMs(retryIndex, retry), signal);
  } catch (cause) {
    throw new NetworkError('API request was aborted', requestId, cause, true);
  }
  throwIfAborted(signal, requestId);
}

function abortableSleep(delayMs: number, signal: AbortSignal | null): Promise<void> {
  if (signal === null) {
    return new Promise((resolve) => {
      globalThis.setTimeout(resolve, delayMs);
    });
  }
  if (signal.aborted) {
    return Promise.reject(abortReason(signal));
  }
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      globalThis.clearTimeout(timer);
      reject(abortReason(signal));
    };
    const timer = globalThis.setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, delayMs);
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error
    ? signal.reason
    : new Error('API request was aborted', { cause: signal.reason });
}

export function throwIfAborted(signal: AbortSignal, requestId: string): void {
  if (signal.aborted) {
    throw new NetworkError('API request was aborted', requestId, signal.reason, true);
  }
}

export function networkError(
  cause: unknown,
  requestId: string,
  signal: AbortSignal | null,
): NetworkError {
  const aborted = signal?.aborted === true || isAbortError(cause);
  return new NetworkError(
    aborted ? 'API request was aborted' : 'API network request failed',
    requestId,
    cause,
    aborted,
  );
}

export function isAbortError(value: unknown): boolean {
  return value instanceof Error && value.name === 'AbortError';
}
