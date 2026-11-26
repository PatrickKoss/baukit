import { SyncRateLimitError, SyncTransportError } from './error.js';

/** Default retry budget before the scheduler waits for the next trigger. */
export const DEFAULT_SYNC_MAX_RETRIES = 5;
const DEFAULT_BASE_DELAY_MS = 1_000;
const DEFAULT_MAX_DELAY_MS = 60_000;
const EXPONENT_BASE = 2;
const SYMMETRIC_SPAN = 2;
/** Recovery triggers (foreground, online) wait a random delay up to this bound. */
export const RECOVERY_JITTER_MAX_MS = 5_000;
/** The periodic interval varies by up to this fraction in either direction. */
export const INTERVAL_JITTER_RATIO = 0.1;

/** Random source in `[0, 1]`; values outside are clamped. */
export type RandomSource = () => number;

function sample(random: RandomSource): number {
  return Math.max(0, Math.min(1, random()));
}

/** Full jitter: a delay between 0 and `min(max, base * 2 ** retryIndex)`. */
export function fullJitterDelayMs(retryIndex: number, random: RandomSource): number {
  const ceiling = Math.min(
    DEFAULT_MAX_DELAY_MS,
    DEFAULT_BASE_DELAY_MS * EXPONENT_BASE ** retryIndex,
  );
  return ceiling * sample(random);
}

/**
 * The default retry delay: `null` for non-retryable transport errors, the time until `retryAt`
 * for a rate limit, otherwise full jitter.
 */
export function defaultSyncRetryDelayMs(
  error: unknown,
  retryIndex: number,
  random: RandomSource,
  now: () => number,
): number | null {
  if (error instanceof SyncRateLimitError) {
    const retryAt = Date.parse(error.retryAt);
    if (!Number.isNaN(retryAt)) {
      return Math.max(0, retryAt - now());
    }
  }
  if (error instanceof SyncTransportError && !error.retryable) {
    return null;
  }
  return fullJitterDelayMs(retryIndex, random);
}

export function recoveryDelayMs(random: RandomSource): number {
  return RECOVERY_JITTER_MAX_MS * sample(random);
}

export function jitteredIntervalMs(intervalMs: number, random: RandomSource): number {
  const offset = (SYMMETRIC_SPAN * sample(random) - 1) * INTERVAL_JITTER_RATIO;
  return intervalMs * (1 + offset);
}
