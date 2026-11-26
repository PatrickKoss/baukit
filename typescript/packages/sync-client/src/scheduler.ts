import {
  DEFAULT_SYNC_MAX_RETRIES,
  defaultSyncRetryDelayMs,
  jitteredIntervalMs,
  recoveryDelayMs,
  type RandomSource,
} from './scheduler-policy.js';

/**
 * Host capabilities the scheduler needs. Products supply timers, foreground
 * state, and connectivity; baukit never reaches for a global.
 */
export interface SyncSchedulerEnvironment {
  isActive(): boolean;
  subscribeActive(listener: (active: boolean) => void): () => void;
  subscribeOnline(listener: () => void): () => void;
  setInterval(callback: () => void, milliseconds: number): SyncSchedulerTimer;
  clearInterval(handle: SyncSchedulerTimer): void;
}

/**
 * Opaque timer handle returned by the host's `setInterval`. Baukit only stores
 * it and hands it back to `clearInterval`; its runtime shape is host-defined.
 */
export type SyncSchedulerTimer = { readonly __syncSchedulerTimer?: never } & object;

/** A retry the scheduler is about to wait for. */
export interface SyncSchedulerRetry {
  readonly error: unknown;
  /** Counts retries since the last success or wake-up, from 0. */
  readonly retryIndex: number;
  readonly delayMs: number;
}

/** Retries a failed run after a delay. */
export interface SyncSchedulerRetryOptions {
  /** Retries after a failed run before the scheduler waits for the next trigger. Defaults to 5. */
  readonly maxRetries?: number;
  /**
   * The delay before retry `retryIndex`, or `null` when the failure must not be retried.
   * Defaults to `retryAt` for `SyncRateLimitError`, no retry for non-retryable transport errors,
   * and full jitter (1 s doubling to 60 s) otherwise.
   */
  readonly delayMs?: (error: unknown, retryIndex: number) => number | null;
  /** Runs before each wait, for example to show when the next attempt starts. */
  readonly onRetryScheduled?: (retry: SyncSchedulerRetry) => void;
}

export interface SyncSchedulerOptions {
  intervalMs?: number;
  onError?: (error: unknown) => void;
  onRecoverySignal?: (signal: SyncSchedulerRecoverySignal) => void;
  /** Retry policy for failed runs. On by default; `false` disables retries. */
  retry?: SyncSchedulerRetryOptions | false;
  /** Random source in `[0, 1]` for retry, recovery and interval jitter. Defaults to `Math.random`. */
  random?: RandomSource;
  /** Epoch-millisecond clock used to honour `SyncRateLimitError.retryAt`. Defaults to `Date.now`. */
  now?: () => number;
}

interface ResolvedRetry {
  readonly maxRetries: number;
  readonly delayMs: (error: unknown, retryIndex: number) => number | null;
  readonly onRetryScheduled: ((retry: SyncSchedulerRetry) => void) | undefined;
}

export type SyncSchedulerRecoverySignal = 'active' | 'online';

type RetryWake = 'elapsed' | 'triggered' | 'stopped';

const DEFAULT_INTERVAL_MS = 5 * 60 * 1000;

function noop(): void {
  return;
}

function resolveRetry(
  retry: SyncSchedulerRetryOptions | false | undefined,
  random: RandomSource,
  now: () => number,
): ResolvedRetry | undefined {
  if (retry === false) {
    return undefined;
  }
  const maxRetries = retry?.maxRetries ?? DEFAULT_SYNC_MAX_RETRIES;
  if (!Number.isInteger(maxRetries) || maxRetries < 0) {
    throw new RangeError('retry.maxRetries must be a non-negative integer');
  }
  return {
    maxRetries,
    delayMs:
      retry?.delayMs ??
      ((error, retryIndex) => defaultSyncRetryDelayMs(error, retryIndex, random, now)),
    onRetryScheduled: retry?.onRetryScheduled,
  };
}

/**
 * Runs one opaque sync callback at most once at a time.
 *
 * A trigger that arrives while a run is active joins that run. A follow-up
 * requested during a run replays the callback once after it settles, so writes
 * made mid-run are never left unsent without starting a parallel run. A
 * failed run is retried after a jittered delay; a trigger during that delay
 * ends it and restarts the retry count. Foreground and online triggers wait a
 * random 0-5 s, and the periodic interval varies by 10%, so clients that
 * recover together do not sync together.
 */
export class SyncScheduler {
  private readonly intervalMs: number;
  private readonly onError: (error: unknown) => void;
  private readonly onRecoverySignal: (signal: SyncSchedulerRecoverySignal) => void;
  private readonly retry: ResolvedRetry | undefined;
  private readonly random: RandomSource;
  private recoveryTimer: SyncSchedulerTimer | null = null;
  private active = false;
  private started = false;
  private inFlight: Promise<void> | null = null;
  private rerunRequested = false;
  private interval: SyncSchedulerTimer | null = null;
  private subscriptions: (() => void)[] = [];
  private wakeRetry: ((reason: RetryWake) => void) | null = null;
  private stops = 0;

  constructor(
    private readonly run: () => Promise<unknown>,
    private readonly environment: SyncSchedulerEnvironment,
    options: SyncSchedulerOptions = {},
  ) {
    this.random = options.random ?? Math.random;
    this.retry = resolveRetry(options.retry, this.random, options.now ?? Date.now);
    this.intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
    this.onError = options.onError ?? noop;
    this.onRecoverySignal = options.onRecoverySignal ?? noop;
  }

  start(): void {
    if (this.started) {
      return;
    }
    this.started = true;
    this.active = this.environment.isActive();
    this.subscriptions = [
      this.environment.subscribeActive((active) => {
        this.active = active;
        this.refreshInterval();
        if (active) {
          this.onRecoverySignal('active');
          this.triggerAfterRecoveryJitter();
        }
      }),
      this.environment.subscribeOnline(() => {
        this.onRecoverySignal('online');
        if (this.active) {
          this.triggerAfterRecoveryJitter();
        }
      }),
    ];
    this.refreshInterval();
    if (this.active) {
      void this.trigger();
    }
  }

  stop(): void {
    this.started = false;
    this.stops += 1;
    this.rerunRequested = false;
    this.wakeRetry?.('stopped');
    this.stopInterval();
    this.cancelRecovery();
    this.subscriptions.forEach((unsubscribe) => {
      unsubscribe();
    });
    this.subscriptions = [];
  }

  /** Starts a run, or joins the active one and ends its retry delay. */
  trigger(): Promise<void> {
    if (this.inFlight) {
      this.wakeRetry?.('triggered');
      return this.inFlight;
    }
    const task: Promise<void> = Promise.resolve()
      .then(() => this.runUntilSettled())
      .finally(() => {
        if (this.inFlight === task) {
          this.inFlight = null;
        }
      });
    this.inFlight = task;
    return task;
  }

  /** Coalesces recovery signals into one trigger after a random delay. */
  private triggerAfterRecoveryJitter(): void {
    if (this.recoveryTimer !== null) {
      return;
    }
    const delayMs = recoveryDelayMs(this.random);
    if (delayMs === 0) {
      void this.trigger();
      return;
    }
    this.recoveryTimer = this.environment.setInterval(() => {
      this.cancelRecovery();
      void this.trigger();
    }, delayMs);
  }

  private cancelRecovery(): void {
    if (this.recoveryTimer === null) {
      return;
    }
    this.environment.clearInterval(this.recoveryTimer);
    this.recoveryTimer = null;
  }

  /** Queues exactly one more run when a run is already active. */
  requestFollowUp(): Promise<void> {
    if (this.inFlight) {
      this.rerunRequested = true;
      return this.inFlight;
    }
    return this.trigger();
  }

  private async runUntilSettled(): Promise<void> {
    const stopsAtStart = this.stops;
    let retryIndex = 0;
    for (;;) {
      this.rerunRequested = false;
      const failure = await this.runOnce();
      if (failure === null) {
        retryIndex = 0;
        if (!this.followUpRequested()) return;
        continue;
      }
      const delayMs = this.retryDelay(failure.error, retryIndex);
      if (delayMs === null) {
        if (!this.followUpRequested()) return;
        continue;
      }
      this.retry?.onRetryScheduled?.({ error: failure.error, retryIndex, delayMs });
      const wake = await this.waitForRetry(delayMs);
      const stopped = this.stops !== stopsAtStart;
      if (stopped || (wake === 'elapsed' && this.started && !this.active)) {
        return;
      }
      retryIndex = wake === 'triggered' ? 0 : retryIndex + 1;
    }
  }

  /** Read through a method: a follow-up can arrive while `run` is awaited. */
  private followUpRequested(): boolean {
    return this.rerunRequested;
  }

  private async runOnce(): Promise<{ readonly error: unknown } | null> {
    try {
      await this.run();
      return null;
    } catch (error) {
      this.onError(error);
      return { error };
    }
  }

  private retryDelay(error: unknown, retryIndex: number): number | null {
    if (this.retry === undefined || retryIndex >= this.retry.maxRetries) {
      return null;
    }
    let delayMs: number | null;
    try {
      delayMs = this.retry.delayMs(error, retryIndex);
    } catch (delayError) {
      this.onError(delayError);
      return null;
    }
    if (delayMs !== null && (!Number.isFinite(delayMs) || delayMs < 0)) {
      this.onError(new RangeError('retry.delayMs must return a finite non-negative number'));
      return null;
    }
    return delayMs;
  }

  /** The environment has interval timers only, so the delay clears its interval on the first tick. */
  private waitForRetry(delayMs: number): Promise<RetryWake> {
    return new Promise((resolve) => {
      const wake = (reason: RetryWake) => {
        this.environment.clearInterval(timer);
        if (this.wakeRetry === wake) {
          this.wakeRetry = null;
        }
        resolve(reason);
      };
      const timer = this.environment.setInterval(() => {
        wake('elapsed');
      }, delayMs);
      this.wakeRetry = wake;
    });
  }

  private refreshInterval(): void {
    this.stopInterval();
    if (!this.started || !this.active) {
      return;
    }
    this.interval = this.environment.setInterval(
      () => {
        this.refreshInterval();
        void this.trigger();
      },
      jitteredIntervalMs(this.intervalMs, this.random),
    );
  }

  private stopInterval(): void {
    if (this.interval === null) {
      return;
    }
    this.environment.clearInterval(this.interval);
    this.interval = null;
  }
}
