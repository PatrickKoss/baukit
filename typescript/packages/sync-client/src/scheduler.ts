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

/** Retries a failed run after a delay the product chooses. */
export interface SyncSchedulerRetryOptions {
  /** Retries after a failed run before the scheduler waits for the next trigger. */
  readonly maxRetries: number;
  /**
   * The delay before retry `retryIndex`, or `null` when the failure must not be retried.
   * `fullJitterBackoffMs` from `@baukit/api-runtime/backoff` gives bounded, jittered delays.
   */
  readonly delayMs: (error: unknown, retryIndex: number) => number | null;
  /** Runs before each wait, for example to show when the next attempt starts. */
  readonly onRetryScheduled?: (retry: SyncSchedulerRetry) => void;
}

export interface SyncSchedulerOptions {
  intervalMs?: number;
  onError?: (error: unknown) => void;
  onRecoverySignal?: (signal: SyncSchedulerRecoverySignal) => void;
  retry?: SyncSchedulerRetryOptions;
}

export type SyncSchedulerRecoverySignal = 'active' | 'online';

type RetryWake = 'elapsed' | 'triggered' | 'stopped';

const DEFAULT_INTERVAL_MS = 5 * 60 * 1000;

function noop(): void {
  return;
}

function validateRetry(retry: SyncSchedulerRetryOptions | undefined): void {
  if (retry === undefined) {
    return;
  }
  if (!Number.isInteger(retry.maxRetries) || retry.maxRetries < 0) {
    throw new RangeError('retry.maxRetries must be a non-negative integer');
  }
}

/**
 * Runs one opaque sync callback at most once at a time.
 *
 * A trigger that arrives while a run is active joins that run. A follow-up
 * requested during a run replays the callback once after it settles, so writes
 * made mid-run are never left unsent without starting a parallel run. With
 * `retry`, a failed run is retried after the chosen delay; a trigger during
 * that delay ends it and restarts the retry count.
 */
export class SyncScheduler {
  private readonly intervalMs: number;
  private readonly onError: (error: unknown) => void;
  private readonly onRecoverySignal: (signal: SyncSchedulerRecoverySignal) => void;
  private readonly retry: SyncSchedulerRetryOptions | undefined;
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
    validateRetry(options.retry);
    this.intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
    this.onError = options.onError ?? noop;
    this.onRecoverySignal = options.onRecoverySignal ?? noop;
    this.retry = options.retry;
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
          void this.trigger();
        }
      }),
      this.environment.subscribeOnline(() => {
        this.onRecoverySignal('online');
        if (this.active) {
          void this.trigger();
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
    this.interval = this.environment.setInterval(() => {
      void this.trigger();
    }, this.intervalMs);
  }

  private stopInterval(): void {
    if (this.interval === null) {
      return;
    }
    this.environment.clearInterval(this.interval);
    this.interval = null;
  }
}
