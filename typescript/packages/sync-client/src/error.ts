/** Machine-readable failure metadata. Products own any message shown to users. */
export type SyncFailure =
  | { readonly kind: 'auth' }
  | { readonly kind: 'partition_mismatch' }
  | { readonly kind: 'rate_limited'; readonly retryAt: string }
  | { readonly kind: 'network' }
  | { readonly kind: 'server' }
  | { readonly kind: 'payload_compatibility' }
  | { readonly kind: 'local_apply' };

/** Snake-case projection of {@link SyncFailure}. */
export type SnakeCaseSyncFailure =
  | Exclude<SyncFailure, { readonly kind: 'rate_limited' }>
  | { readonly kind: 'rate_limited'; readonly retry_at: string };

/** A sync request failed. `retryable` decides whether a later attempt may win. */
export class SyncTransportError extends Error {
  constructor(
    message: string,
    readonly retryable: boolean,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'SyncTransportError';
  }
}

/** Credentials were rejected. Never retryable without re-authentication. */
export class SyncAuthError extends SyncTransportError {
  readonly kind = 'auth';

  constructor(message: string, cause?: unknown) {
    super(message, false, cause);
    this.name = 'SyncAuthError';
  }
}

/** The request did not reach a usable HTTP response. */
export class SyncNetworkError extends SyncTransportError {
  readonly kind = 'network';

  constructor(message: string, cause?: unknown) {
    super(message, true, cause);
    this.name = 'SyncNetworkError';
  }
}

/** The server returned an HTTP failure other than authentication or rate limiting. */
export class SyncServerError extends SyncTransportError {
  readonly kind = 'server';

  constructor(message: string, retryable: boolean, cause?: unknown) {
    super(message, retryable, cause);
    this.name = 'SyncServerError';
  }
}

/** HTTP 429 with its raw `Retry-After` value and resolved retry time. */
export class SyncRateLimitError extends SyncTransportError {
  readonly kind = 'rate_limited';

  constructor(
    message: string,
    readonly retryAt: string,
    readonly retryAfter: string | null,
    cause?: unknown,
  ) {
    super(message, true, cause);
    this.name = 'SyncRateLimitError';
  }
}

/**
 * The server rejected the caller's local-data partition, so the local database
 * belongs to an erased or replaced owner and must not be reused.
 */
export class SyncPartitionMismatchError extends SyncTransportError {
  readonly kind = 'partition_mismatch';

  constructor(message: string, cause?: unknown) {
    super(message, false, cause);
    this.name = 'SyncPartitionMismatchError';
  }
}

/** A decoded response violates the sync payload contract. */
export class SyncPayloadCompatibilityError extends SyncTransportError {
  readonly kind = 'payload_compatibility';

  constructor(message: string, cause?: unknown) {
    super(message, false, cause);
    this.name = 'SyncPayloadCompatibilityError';
  }
}

/** Remote data or a cursor could not be committed to local storage. */
export class SyncLocalApplyError extends Error {
  readonly kind = 'local_apply';

  constructor(message: string, cause?: unknown) {
    super(message, { cause });
    this.name = 'SyncLocalApplyError';
  }
}

/** Maps package errors onto store failure metadata. */
export function syncFailureFromError(error: unknown): SyncFailure {
  if (error instanceof SyncRateLimitError) {
    return { kind: 'rate_limited', retryAt: error.retryAt };
  }
  if (error instanceof SyncAuthError) return { kind: 'auth' };
  if (error instanceof SyncPartitionMismatchError) return { kind: 'partition_mismatch' };
  if (error instanceof SyncNetworkError) return { kind: 'network' };
  if (error instanceof SyncServerError) return { kind: 'server' };
  if (error instanceof SyncPayloadCompatibilityError) return { kind: 'payload_compatibility' };
  if (error instanceof SyncLocalApplyError) return { kind: 'local_apply' };
  if (error instanceof SyncTransportError) {
    return error.retryable ? { kind: 'network' } : { kind: 'server' };
  }
  return { kind: 'local_apply' };
}

export function toSnakeCaseFailure(failure: SyncFailure): SnakeCaseSyncFailure {
  return failure.kind === 'rate_limited'
    ? { kind: failure.kind, retry_at: failure.retryAt }
    : failure;
}
