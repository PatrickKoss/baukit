import { describe, expect, it, vi } from 'vitest';

import { SyncAuthError, SyncRateLimitError } from './error.js';
import {
  SyncScheduler,
  type SyncSchedulerEnvironment,
  type SyncSchedulerRetry,
  type SyncSchedulerTimer,
} from './scheduler.js';

const INTERVAL_MS = 10_000;
const NOW = Date.UTC(2026, 0, 1);

interface Timers extends SyncSchedulerEnvironment {
  goOnline(): void;
  setActive(active: boolean): void;
  pending(): number[];
  fire(ms: number): void;
}

function environment(initiallyActive = true): Timers {
  let active = initiallyActive;
  const activeListeners = new Set<(next: boolean) => void>();
  const onlineListeners = new Set<() => void>();
  const timers = new Map<SyncSchedulerTimer, { callback: () => void; ms: number }>();
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
    goOnline() {
      onlineListeners.forEach((listener) => {
        listener();
      });
    },
    setActive(next) {
      active = next;
      activeListeners.forEach((listener) => {
        listener(next);
      });
    },
    pending: () => [...timers.values()].map((timer) => timer.ms),
    fire(ms) {
      [...timers.values()]
        .filter((timer) => timer.ms === ms)
        .forEach(({ callback }) => {
          callback();
        });
    },
  };
}

const settle = async () => {
  for (let tick = 0; tick < 5; tick += 1) {
    await Promise.resolve();
  }
};

function failing(error: Error, failures = 1) {
  let calls = 0;
  return vi.fn(() => {
    calls += 1;
    return calls <= failures ? Promise.reject(error) : Promise.resolve();
  });
}

describe('SyncScheduler defaults', () => {
  it('retries a failed run with full jitter when the product sets no retry', async () => {
    const env = environment(false);
    const scheduled: SyncSchedulerRetry[] = [];
    const run = failing(new Error('offline'), 2);
    const scheduler = new SyncScheduler(run, env, {
      random: () => 0.5,
      retry: { onRetryScheduled: (retry) => scheduled.push(retry) },
    });

    const done = scheduler.trigger();
    await settle();
    env.fire(500);
    await settle();
    env.fire(1_000);
    await done;

    expect(run).toHaveBeenCalledTimes(3);
    expect(scheduled.map((retry) => retry.delayMs)).toEqual([500, 1_000]);
  });

  it('waits until SyncRateLimitError.retryAt', async () => {
    const env = environment(false);
    const retryAt = new Date(NOW + 7_000).toISOString();
    const run = failing(new SyncRateLimitError('slow down', retryAt, '7'));
    const scheduler = new SyncScheduler(run, env, { now: () => NOW });

    const done = scheduler.trigger();
    await settle();
    expect(env.pending()).toEqual([7_000]);
    env.fire(7_000);
    await done;
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('does not retry a non-retryable transport error', async () => {
    const env = environment(false);
    const run = failing(new SyncAuthError('expired'));
    await new SyncScheduler(run, env).trigger();

    expect(run).toHaveBeenCalledTimes(1);
    expect(env.pending()).toEqual([]);
  });

  it('does not retry when retry is false', async () => {
    const env = environment(false);
    const run = failing(new Error('offline'));
    await new SyncScheduler(run, env, { retry: false }).trigger();

    expect(run).toHaveBeenCalledTimes(1);
    expect(env.pending()).toEqual([]);
  });

  it('delays recovery triggers by a random 0-5 s and coalesces them', async () => {
    const env = environment(false);
    const run = vi.fn(() => Promise.resolve());
    const scheduler = new SyncScheduler(run, env, { intervalMs: INTERVAL_MS, random: () => 0.4 });
    scheduler.start();

    env.setActive(true);
    env.goOnline();
    await settle();
    expect(run).not.toHaveBeenCalled();
    expect(env.pending().filter((ms) => ms === 2_000)).toHaveLength(1);

    env.fire(2_000);
    await settle();
    expect(run).toHaveBeenCalledTimes(1);
    expect(env.pending()).not.toContain(2_000);
    scheduler.stop();
  });

  it('cancels a pending recovery trigger on stop', async () => {
    const env = environment(false);
    const run = vi.fn(() => Promise.resolve());
    const scheduler = new SyncScheduler(run, env, { random: () => 1 });
    scheduler.start();
    env.setActive(true);
    scheduler.stop();
    await settle();

    expect(env.pending()).toEqual([]);
    expect(run).not.toHaveBeenCalled();
  });

  it('jitters the periodic interval by 10% and re-arms it on every tick', async () => {
    const env = environment();
    const samples = [0, 1];
    const run = vi.fn(() => Promise.resolve());
    const scheduler = new SyncScheduler(run, env, {
      intervalMs: INTERVAL_MS,
      random: () => samples.shift() ?? 0.5,
    });

    scheduler.start();
    await settle();
    expect(env.pending()).toEqual([9_000]);

    env.fire(9_000);
    await settle();
    expect(env.pending()).toEqual([11_000]);
    expect(run).toHaveBeenCalledTimes(2);
    scheduler.stop();
  });
});
