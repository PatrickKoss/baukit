/** Exponential backoff with full jitter. The retry index counts from 0. */
export interface BackoffPolicy {
  /** Ceiling for the first retry, doubled for each later one. */
  readonly baseDelayMs: number;
  /** The ceiling never grows past this. */
  readonly maxDelayMs: number;
  /** Random source in `[0, 1]`. Values outside are clamped. Defaults to `Math.random`. */
  readonly random?: () => number;
}

const EXPONENT_BASE = 2;

/**
 * Returns a delay between 0 and `min(maxDelayMs, baseDelayMs * 2 ** retryIndex)`.
 *
 * Full jitter spreads clients that failed together, so they do not retry together.
 */
export function fullJitterBackoffMs(retryIndex: number, policy: BackoffPolicy): number {
  const ceiling = Math.min(policy.maxDelayMs, policy.baseDelayMs * EXPONENT_BASE ** retryIndex);
  const sample = (policy.random ?? Math.random)();
  return ceiling * Math.max(0, Math.min(1, sample));
}
