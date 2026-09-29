import { describe, expect, it, vi } from 'vitest';

import {
  SyncScheduler,
  type SyncSchedulerEnvironment,
  type SyncSchedulerRetry,
  type SyncSchedulerRetryOptions,
  type SyncSchedulerTimer,
} from './scheduler.js';

const INTERVAL_MS = 60_000;

interface Timer {
  readonly callback: () => void;
  readonly ms: number;
}

interface RetryEnvironment extends SyncSchedulerEnvironment {
  setActive(active: boolean): void;
  goOnline(): void;
  /** Fires the pending timers that are not the periodic interval. */
  elapseRetryDelay(): void;
  retryDelays(): number[];
}

function retryEnvironment(): RetryEnvironment {
  let active = true;
  const activeListeners = new Set<(next: boolean) => void>();
  const onlineListeners = new Set<() => void>();
  const timers = new Map<SyncSchedulerTimer, Timer>();
  const retryTimers = () => [...timers.values()].filter((timer) => timer.ms !== INTERVAL_MS);

  return {
    isActive: () => active,
    subscribeActive(listener) {
      activeListeners.add(listener);
      return () => activeListeners.delete(listener);
    },
    subscribeOnline(listener) {
      onlineListeners.add(listener);
      return () => onlineListeners.delete(listener);
    },
    setInterval(callback, ms) {
      const handle = {};
      timers.set(handle, { callback, ms });
      return handle;
    },
    clearInterval(handle) {
      timers.delete(handle);
    },
    setActive(next) {
      active = next;
      activeListeners.forEach((listener) => {
        listener(next);
      });
    },
    goOnline() {
      onlineListeners.forEach((listener) => {
        listener();
      });
    },
    elapseRetryDelay() {
      retryTimers().forEach(({ callback }) => {
        callback();
      });
    },
    retryDelays: () => retryTimers().map((timer) => timer.ms),
  };
}

function failingRun(failures: number) {
  let calls = 0;
  return {
    run: vi.fn(() => {
      calls += 1;
      return calls <= failures
        ? Promise.reject(new Error(`attempt ${String(calls)} failed`))
        : Promise.resolve();
    }),
  };
}

const settle = async () => {
  for (let tick = 0; tick < 5; tick += 1) {
    await Promise.resolve();
  }
};

function retryOptions(overrides: Partial<SyncSchedulerRetryOptions> = {}) {
  const scheduled: SyncSchedulerRetry[] = [];
  const options: SyncSchedulerRetryOptions = {
    maxRetries: 3,
    delayMs: (_error, retryIndex) => 100 * 2 ** retryIndex,
    onRetryScheduled: (retry) => {
      scheduled.push(retry);
    },
    ...overrides,
  };
  return { options, scheduled };
}

describe('SyncScheduler retry', () => {
  it('retries a failed run after the chosen delays until it succeeds', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(2);
    const onError = vi.fn();
    const { options, scheduled } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, {
      intervalMs: INTERVAL_MS,
      onError,
      retry: options,
    });

    const done = scheduler.trigger();
    await settle();
    expect(environment.retryDelays()).toEqual([100]);

    environment.elapseRetryDelay();
    await settle();
    expect(environment.retryDelays()).toEqual([200]);

    environment.elapseRetryDelay();
    await done;

    expect(run).toHaveBeenCalledTimes(3);
    expect(onError).toHaveBeenCalledTimes(2);
    expect(scheduled.map(({ retryIndex, delayMs }) => [retryIndex, delayMs])).toEqual([
      [0, 100],
      [1, 200],
    ]);
    expect(environment.retryDelays()).toEqual([]);
  });

  it('stops after maxRetries and waits for the next trigger', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(10);
    const { options } = retryOptions({ maxRetries: 1 });
    const scheduler = new SyncScheduler(run, environment, { retry: options });

    const done = scheduler.trigger();
    await settle();
    environment.elapseRetryDelay();
    await done;

    expect(run).toHaveBeenCalledTimes(2);
    expect(environment.retryDelays()).toEqual([]);
  });

  it('does not retry when the delay is null', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(1);
    const { options } = retryOptions({ delayMs: () => null });

    await new SyncScheduler(run, environment, { retry: options }).trigger();

    expect(run).toHaveBeenCalledTimes(1);
  });

  it('passes the failure to the delay function', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(1);
    const delayMs = vi.fn(() => null);

    await new SyncScheduler(run, environment, { retry: { maxRetries: 2, delayMs } }).trigger();

    expect(delayMs).toHaveBeenCalledWith(new Error('attempt 1 failed'), 0);
  });

  it('ends the delay and restarts the count when a trigger arrives', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(3);
    const { options, scheduled } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, { retry: options });

    const done = scheduler.trigger();
    await settle();
    environment.elapseRetryDelay();
    await settle();
    expect(scheduled.at(-1)?.retryIndex).toBe(1);

    const joined = scheduler.trigger();
    await settle();

    expect(joined).toBe(done);
    expect(scheduled.at(-1)?.retryIndex).toBe(0);
    environment.elapseRetryDelay();
    await done;
    expect(run).toHaveBeenCalledTimes(4);
  });

  it('retries at once when the app returns to the foreground or connectivity returns', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(2);
    const signals: string[] = [];
    const { options } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, {
      intervalMs: INTERVAL_MS,
      retry: options,
      onRecoverySignal: (signal) => signals.push(signal),
    });

    scheduler.start();
    await settle();
    environment.goOnline();
    await settle();
    expect(run).toHaveBeenCalledTimes(2);

    environment.setActive(false);
    environment.setActive(true);
    await settle();

    expect(run).toHaveBeenCalledTimes(3);
    expect(signals).toEqual(['online', 'active']);
    expect(environment.retryDelays()).toEqual([]);
    scheduler.stop();
  });

  it('stops retrying when the app is backgrounded during the delay', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(5);
    const { options } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, {
      intervalMs: INTERVAL_MS,
      retry: options,
    });

    scheduler.start();
    await settle();
    environment.setActive(false);
    environment.elapseRetryDelay();
    await settle();

    expect(run).toHaveBeenCalledTimes(1);
    scheduler.stop();
  });

  it('ends the delay without another run when scheduling stops', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(5);
    const { options } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, {
      intervalMs: INTERVAL_MS,
      retry: options,
    });

    scheduler.start();
    await settle();
    const pending = scheduler.trigger();
    scheduler.stop();
    await pending;

    expect(run).toHaveBeenCalledTimes(1);
    expect(environment.retryDelays()).toEqual([]);
  });

  it('covers a follow-up with the retry instead of an immediate rerun', async () => {
    const environment = retryEnvironment();
    const { run } = failingRun(1);
    const { options } = retryOptions();
    const scheduler = new SyncScheduler(run, environment, { retry: options });

    const done = scheduler.trigger();
    void scheduler.requestFollowUp();
    await settle();
    expect(run).toHaveBeenCalledTimes(1);

    environment.elapseRetryDelay();
    await done;
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('reports a delay function that throws or returns an invalid delay, and stops retrying', async () => {
    const environment = retryEnvironment();
    const onError = vi.fn();
    const broken = new Error('delay failed');

    await new SyncScheduler(failingRun(1).run, environment, {
      onError,
      retry: {
        maxRetries: 1,
        delayMs: () => {
          throw broken;
        },
      },
    }).trigger();
    await new SyncScheduler(failingRun(1).run, environment, {
      onError,
      retry: { maxRetries: 1, delayMs: () => Number.NaN },
    }).trigger();

    expect(onError).toHaveBeenCalledWith(broken);
    expect(onError).toHaveBeenLastCalledWith(expect.any(RangeError));
    expect(environment.retryDelays()).toEqual([]);
  });

  it('rejects a maxRetries that is not a non-negative integer', () => {
    const environment = retryEnvironment();
    const run = () => Promise.resolve();

    expect(
      () => new SyncScheduler(run, environment, { retry: { maxRetries: -1, delayMs: () => 0 } }),
    ).toThrow(RangeError);
    expect(
      () => new SyncScheduler(run, environment, { retry: { maxRetries: 1.5, delayMs: () => 0 } }),
    ).toThrow(RangeError);
  });
});
