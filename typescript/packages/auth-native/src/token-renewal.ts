import { isTerminalRefreshError } from './oidc-support.js';

const DEFAULT_LEAD_MS = 30_000;
const DEFAULT_BASE_DELAY_MS = 1_000;
const DEFAULT_MAX_DELAY_MS = 60_000;
const EXPONENT_BASE = 2;

export interface TokenRenewalOptions {
  /** Epoch milliseconds at which the current access token expires. */
  readonly expiresAt: number;
  /** Refreshes the session, for example `() => client.accessToken({ forceRefresh: true })`. */
  readonly renew: () => Promise<unknown>;
  /** Called with every failed attempt, before the next one is scheduled. */
  readonly onError?: (cause: unknown) => void;
  /** How long before `expiresAt` the first attempt runs. Defaults to 30 s. */
  readonly leadMs?: number;
  /** Ceiling of the first retry delay, doubled per retry. Defaults to 1 s. */
  readonly baseDelayMs?: number;
  /** Largest retry delay ceiling. Defaults to 60 s. */
  readonly maxDelayMs?: number;
  readonly now?: () => number;
  /** Random source in `[0, 1)` for full jitter. Defaults to `Math.random`. */
  readonly random?: () => number;
}

/**
 * Renews once shortly before expiry. A failure retries with full-jitter exponential backoff until a
 * renewal succeeds, the provider rejects the refresh token (`invalid_grant`), or the returned cancel runs.
 */
export function scheduleTokenRenewal(options: TokenRenewalOptions): () => void {
  const now = options.now ?? Date.now;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let cancelled = false;

  const attempt = (retryIndex: number): void => {
    timer = undefined;
    void options.renew().then(
      () => undefined,
      (cause: unknown) => {
        if (cancelled) return;
        options.onError?.(cause);
        if (isTerminalRefreshError(cause)) return;
        timer = setTimeout(
          () => {
            attempt(retryIndex + 1);
          },
          retryDelayMs(retryIndex, options),
        );
      },
    );
  };

  const firstDelay = Math.max(options.expiresAt - now() - (options.leadMs ?? DEFAULT_LEAD_MS), 0);
  timer = setTimeout(() => {
    attempt(0);
  }, firstDelay);

  return () => {
    cancelled = true;
    if (timer !== undefined) clearTimeout(timer);
  };
}

function retryDelayMs(retryIndex: number, options: TokenRenewalOptions): number {
  const base = options.baseDelayMs ?? DEFAULT_BASE_DELAY_MS;
  const ceiling = Math.min(
    options.maxDelayMs ?? DEFAULT_MAX_DELAY_MS,
    base * EXPONENT_BASE ** retryIndex,
  );
  const sample = Math.min(Math.max((options.random ?? Math.random)(), 0), 1);
  return Math.floor(ceiling * sample);
}
