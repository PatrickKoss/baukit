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

export interface SyncSchedulerOptions {
  intervalMs?: number;
  onError?: (error: unknown) => void;
  onRecoverySignal?: (signal: SyncSchedulerRecoverySignal) => void;
}

export type SyncSchedulerRecoverySignal = 'active' | 'online';

const DEFAULT_INTERVAL_MS = 5 * 60 * 1000;

function noop(): void {
  return;
}

/**
 * Runs one opaque sync callback at most once at a time.
 *
 * A trigger that arrives while a run is active joins that run. A follow-up
 * requested during a run replays the callback once after it settles, so writes
 * made mid-run are never left unsent without starting a parallel run.
 */
export class SyncScheduler {
  private readonly intervalMs: number;
  private readonly onError: (error: unknown) => void;
  private readonly onRecoverySignal: (signal: SyncSchedulerRecoverySignal) => void;
  private active = false;
  private started = false;
  private inFlight: Promise<void> | null = null;
  private rerunRequested = false;
  private interval: SyncSchedulerTimer | null = null;
  private subscriptions: (() => void)[] = [];

  constructor(
    private readonly run: () => Promise<unknown>,
    private readonly environment: SyncSchedulerEnvironment,
    options: SyncSchedulerOptions = {},
  ) {
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
    this.rerunRequested = false;
    this.stopInterval();
    this.subscriptions.forEach((unsubscribe) => {
      unsubscribe();
    });
    this.subscriptions = [];
  }

  /** Starts a run, or joins the active one. */
  trigger(): Promise<void> {
    if (this.inFlight) {
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
    let rerun = true;
    while (rerun) {
      this.rerunRequested = false;
      try {
        await this.run();
      } catch (error) {
        this.onError(error);
      }
      rerun = this.rerunRequested;
    }
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
