/** The request ID header accepted and returned by `baukit-http`. */
export const REQUEST_ID_HEADER = 'x-request-id' as const;

/** The request header that carries a mutation's idempotency key. */
export const IDEMPOTENCY_KEY_HEADER = 'idempotency-key' as const;
