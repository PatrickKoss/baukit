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

/**
 * The server rejected the caller's local-data partition, so the local database
 * belongs to an erased or replaced owner and must not be reused.
 */
export class SyncPartitionMismatchError extends SyncTransportError {
  readonly kind = 'partition-mismatch';

  constructor(message: string, cause?: unknown) {
    super(message, false, cause);
    this.name = 'SyncPartitionMismatchError';
  }
}
