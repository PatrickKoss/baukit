/** Per-origin cooldown options. */
export interface CooldownOptions {
  /** Consecutive network failures (timeouts included) that start a cooldown. Defaults to 5. */
  readonly networkFailureThreshold?: number;
  /** Cooldown started by the network failure threshold, in milliseconds. Defaults to 30,000. */
  readonly networkFailureCooldownMs?: number;
}

const DEFAULT_NETWORK_FAILURE_THRESHOLD = 5;
const DEFAULT_NETWORK_FAILURE_COOLDOWN_MS = 30_000;

interface OriginState {
  consecutiveNetworkFailures: number;
  retryAt: number;
}

/**
 * Tracks when each origin may be called again (plan D9).
 *
 * A `429`/`503` with `Retry-After`, or a run of network failures, blocks the origin until `retryAt`.
 */
export class OriginCooldown {
  readonly #threshold: number;
  readonly #networkCooldownMs: number;
  readonly #now: () => number;
  readonly #origins = new Map<string, OriginState>();

  constructor(options: CooldownOptions, now: () => number) {
    this.#threshold = options.networkFailureThreshold ?? DEFAULT_NETWORK_FAILURE_THRESHOLD;
    this.#networkCooldownMs =
      options.networkFailureCooldownMs ?? DEFAULT_NETWORK_FAILURE_COOLDOWN_MS;
    if (!Number.isInteger(this.#threshold) || this.#threshold < 1) {
      throw new Error('networkFailureThreshold must be a positive integer');
    }
    if (!Number.isFinite(this.#networkCooldownMs) || this.#networkCooldownMs < 0) {
      throw new Error('networkFailureCooldownMs must be a finite non-negative number');
    }
    this.#now = now;
  }

  /** Returns the time the origin unblocks, or null when calls may proceed. */
  blockedUntil(origin: string): number | null {
    const retryAt = this.#origins.get(origin)?.retryAt ?? 0;
    return this.#now() < retryAt ? retryAt : null;
  }

  /** Blocks the origin for `delayMs` from now, never shortening an existing cooldown. */
  coolDown(origin: string, delayMs: number): void {
    const state = this.#state(origin);
    state.retryAt = Math.max(state.retryAt, this.#now() + delayMs);
  }

  recordResponse(origin: string): void {
    const state = this.#origins.get(origin);
    if (state === undefined) {
      return;
    }
    state.consecutiveNetworkFailures = 0;
    if (this.#now() >= state.retryAt) {
      this.#origins.delete(origin);
    }
  }

  recordNetworkFailure(origin: string): void {
    const state = this.#state(origin);
    state.consecutiveNetworkFailures += 1;
    if (state.consecutiveNetworkFailures < this.#threshold) {
      return;
    }
    state.consecutiveNetworkFailures = 0;
    this.coolDown(origin, this.#networkCooldownMs);
  }

  #state(origin: string): OriginState {
    const existing = this.#origins.get(origin);
    if (existing !== undefined) {
      return existing;
    }
    const created: OriginState = { consecutiveNetworkFailures: 0, retryAt: 0 };
    this.#origins.set(origin, created);
    return created;
  }
}
