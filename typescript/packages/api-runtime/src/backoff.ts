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
/** Products saturate here, as in `fixtures/resilience/backoff-v1.json`. */
const SATURATION_MS = 2 ** 32 - 1;

/**
 * Returns `floor(min(maxDelayMs, baseDelayMs * 2 ** retryIndex) * random)` in whole milliseconds.
 *
 * Full jitter spreads clients that failed together, so they do not retry together.
 */
export function fullJitterBackoffMs(retryIndex: number, policy: BackoffPolicy): number {
  const product = Math.min(SATURATION_MS, policy.baseDelayMs * EXPONENT_BASE ** retryIndex);
  const ceiling = Math.min(policy.maxDelayMs, product);
  const sample = (policy.random ?? Math.random)();
  return Math.floor(ceiling * Math.max(0, Math.min(1, sample)));
}
