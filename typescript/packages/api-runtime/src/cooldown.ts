/** Per-origin cooldown options. */
export interface CooldownOptions {
  /** Consecutive network failures (timeouts included) that start a cooldown. Defaults to 5. */
  readonly networkFailureThreshold?: number;
  /** First cooldown started by the network failure threshold, in milliseconds. Defaults to 30,000. */
  readonly networkFailureCooldownMs?: number;
  /** Cap for every cooldown, including doubled ones and `Retry-After` holds. Defaults to 300,000. */
  readonly maxCooldownMs?: number;
  /** Share in percent taken off a network failure cooldown at random. Defaults to 20. */
  readonly jitterPercent?: number;
  /** Random source in `[0, 1]` for the jitter. Values outside are clamped. Defaults to `Math.random`. */
  readonly random?: () => number;
}

const DEFAULT_NETWORK_FAILURE_THRESHOLD = 5;
const DEFAULT_NETWORK_FAILURE_COOLDOWN_MS = 30_000;
const DEFAULT_MAX_COOLDOWN_MS = 300_000;
const DEFAULT_JITTER_PERCENT = 20;
const PERCENT = 100;
const EXPONENT_BASE = 2;

interface OriginState {
  consecutiveNetworkFailures: number;
  networkOpenings: number;
  retryAt: number;
}

function assertNonNegative(value: number, name: string): void {
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`${name} must be a finite non-negative number`);
  }
}

/**
 * Tracks when each origin may be called again (plan D9).
 *
 * A `429`/`503` with `Retry-After`, or a run of network failures, blocks the origin until `retryAt`.
 * Each run of network failures without a response in between doubles the cooldown up to
 * `maxCooldownMs`, minus jitter, matching the open time in `fixtures/resilience/breaker-v1.json`.
 */
export class OriginCooldown {
  readonly #threshold: number;
  readonly #networkCooldownMs: number;
  readonly #maxCooldownMs: number;
  readonly #jitterRatio: number;
  readonly #random: () => number;
  readonly #now: () => number;
  readonly #origins = new Map<string, OriginState>();

  constructor(options: CooldownOptions, now: () => number) {
    this.#threshold = options.networkFailureThreshold ?? DEFAULT_NETWORK_FAILURE_THRESHOLD;
    this.#networkCooldownMs =
      options.networkFailureCooldownMs ?? DEFAULT_NETWORK_FAILURE_COOLDOWN_MS;
    this.#maxCooldownMs = options.maxCooldownMs ?? DEFAULT_MAX_COOLDOWN_MS;
    const jitterPercent = options.jitterPercent ?? DEFAULT_JITTER_PERCENT;
    if (!Number.isInteger(this.#threshold) || this.#threshold < 1) {
      throw new Error('networkFailureThreshold must be a positive integer');
    }
    assertNonNegative(this.#networkCooldownMs, 'networkFailureCooldownMs');
    assertNonNegative(this.#maxCooldownMs, 'maxCooldownMs');
    if (!Number.isFinite(jitterPercent) || jitterPercent < 0 || jitterPercent > PERCENT) {
      throw new Error('jitterPercent must be between 0 and 100');
    }
    this.#jitterRatio = jitterPercent / PERCENT;
    this.#random = options.random ?? Math.random;
    this.#now = now;
  }

  /** Returns the time the origin unblocks, or null when calls may proceed. */
  blockedUntil(origin: string): number | null {
    const retryAt = this.#origins.get(origin)?.retryAt ?? 0;
    return this.#now() < retryAt ? retryAt : null;
  }

  /** Blocks the origin for `delayMs` from now, capped, never shortening an existing cooldown. */
  coolDown(origin: string, delayMs: number): void {
    const state = this.#state(origin);
    const held = Math.min(delayMs, this.#maxCooldownMs);
    state.retryAt = Math.max(state.retryAt, this.#now() + held);
  }

  recordResponse(origin: string): void {
    const state = this.#origins.get(origin);
    if (state === undefined) {
      return;
    }
    state.consecutiveNetworkFailures = 0;
    state.networkOpenings = 0;
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
    state.networkOpenings += 1;
    this.coolDown(origin, this.#networkCooldownFor(state.networkOpenings));
  }

  #networkCooldownFor(opening: number): number {
    const open = Math.min(
      this.#maxCooldownMs,
      this.#networkCooldownMs * EXPONENT_BASE ** (opening - 1),
    );
    const fraction = Math.max(0, Math.min(1, this.#random()));
    return open - Math.floor(open * (this.#jitterRatio * fraction));
  }

  #state(origin: string): OriginState {
    const existing = this.#origins.get(origin);
    if (existing !== undefined) {
      return existing;
    }
    const created: OriginState = { consecutiveNetworkFailures: 0, networkOpenings: 0, retryAt: 0 };
    this.#origins.set(origin, created);
    return created;
  }
}
