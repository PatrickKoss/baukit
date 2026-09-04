import { describe, expect, it, vi } from 'vitest';

import {
  SyncScheduler,
  type SyncSchedulerEnvironment,
  type SyncSchedulerTimer,
} from './scheduler.js';

interface FakeEnvironment extends SyncSchedulerEnvironment {
  setActive(active: boolean): void;
  goOnline(): void;
  fireInterval(): void;
  intervalCount(): number;
  lastIntervalMs(): number | null;
}

function fakeEnvironment(initiallyActive = true): FakeEnvironment {
  let active = initiallyActive;
  const activeListeners = new Set<(next: boolean) => void>();
  const onlineListeners = new Set<() => void>();
  const timers = new Map<SyncSchedulerTimer, { callback: () => void; ms: number }>();
  let nextHandle = 1;
  let lastMs: number | null = null;

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
    setInterval(callback, milliseconds) {
      const handle = nextHandle++;
      lastMs = milliseconds;
      timers.set(handle, { callback, ms: milliseconds });
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
    fireInterval() {
      [...timers.values()].forEach(({ callback }) => {
        callback();
      });
    },
    intervalCount: () => timers.size,
    lastIntervalMs: () => lastMs,
  };
}

function deferredRun(): { run: () => Promise<void>; finish: () => void; calls: () => number } {
  const finishes: (() => void)[] = [];
  let calls = 0;
  return {
    run: () => {
      calls += 1;
      return new Promise<void>((resolve) => finishes.push(resolve));
    },
    finish: () => finishes.shift()?.(),
    calls: () => calls,
  };
}

describe('SyncScheduler', () => {
  it('runs once on start and installs the periodic interval', async () => {
    const environment = fakeEnvironment();
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment, { intervalMs: 1000 });

    scheduler.start();
    await Promise.resolve();

    expect(run).toHaveBeenCalledTimes(1);
    expect(environment.intervalCount()).toBe(1);
    expect(environment.lastIntervalMs()).toBe(1000);
  });

  it('does not start a run or an interval while the app is backgrounded', async () => {
    const environment = fakeEnvironment(false);
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment);

    scheduler.start();
    await Promise.resolve();

    expect(run).not.toHaveBeenCalled();
    expect(environment.intervalCount()).toBe(0);
  });

  it('runs when the app returns to the foreground and when connectivity returns', async () => {
    const environment = fakeEnvironment(false);
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment);

    scheduler.start();
    environment.setActive(true);
    await scheduler.trigger();
    expect(run).toHaveBeenCalledTimes(1);
    expect(environment.intervalCount()).toBe(1);

    environment.goOnline();
    await scheduler.trigger();
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('ignores connectivity while the app is backgrounded', async () => {
    const environment = fakeEnvironment(false);
    const run = vi.fn(async () => Promise.resolve());
    new SyncScheduler(run, environment).start();

    environment.goOnline();
    await Promise.resolve();

    expect(run).not.toHaveBeenCalled();
  });

  it('reports recovery signals before triggering a scheduler run', async () => {
    const environment = fakeEnvironment(false);
    const calls: string[] = [];
    const scheduler = new SyncScheduler(
      () => {
        calls.push('run');
        return Promise.resolve();
      },
      environment,
      { onRecoverySignal: (signal) => calls.push(signal) },
    );
    scheduler.start();

    environment.setActive(true);
    await scheduler.trigger();
    environment.goOnline();
    await scheduler.trigger();

    expect(calls).toEqual(['active', 'run', 'online', 'run']);
  });

  it('reports an online recovery signal while backgrounded without starting a run', async () => {
    const environment = fakeEnvironment(false);
    const onRecoverySignal = vi.fn();
    const run = vi.fn(() => Promise.resolve());
    new SyncScheduler(run, environment, { onRecoverySignal }).start();

    environment.goOnline();
    await Promise.resolve();

    expect(onRecoverySignal).toHaveBeenCalledWith('online');
    expect(run).not.toHaveBeenCalled();
  });

  it('runs on every interval tick', async () => {
    const environment = fakeEnvironment();
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment, { intervalMs: 1000 });

    scheduler.start();
    await scheduler.trigger();
    expect(run).toHaveBeenCalledTimes(1);

    environment.fireInterval();
    await scheduler.trigger();

    expect(run).toHaveBeenCalledTimes(2);
  });

  it('coalesces a manual trigger that overlaps the active run', async () => {
    const { run, finish, calls } = deferredRun();
    const scheduler = new SyncScheduler(run, fakeEnvironment());

    scheduler.start();
    const manual = scheduler.trigger();
    await Promise.resolve();

    expect(calls()).toBe(1);
    finish();
    await manual;
    expect(calls()).toBe(1);
  });

  it('queues exactly one follow-up when writes arrive during a run', async () => {
    const { run, finish, calls } = deferredRun();
    const scheduler = new SyncScheduler(run, fakeEnvironment(false));
    scheduler.start();

    const first = scheduler.trigger();
    await Promise.resolve();
    const second = scheduler.requestFollowUp();
    const third = scheduler.requestFollowUp();
    expect(calls()).toBe(1);

    finish();
    await Promise.resolve();
    await Promise.resolve();
    expect(calls()).toBe(2);

    finish();
    await Promise.all([first, second, third]);
    expect(calls()).toBe(2);
  });

  it('observes a follow-up requested from inside the run without going parallel', async () => {
    const environment = fakeEnvironment(false);
    let active = 0;
    let maximumActive = 0;
    let calls = 0;
    const run = async (): Promise<void> => {
      calls += 1;
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      if (calls === 1) void scheduler.requestFollowUp();
      await Promise.resolve();
      active -= 1;
    };
    const scheduler: SyncScheduler = new SyncScheduler(run, environment);

    await scheduler.trigger();

    expect(calls).toBe(2);
    expect(maximumActive).toBe(1);
  });

  it('drops a queued follow-up when scheduling stops during the run', async () => {
    const { run, finish, calls } = deferredRun();
    const scheduler = new SyncScheduler(run, fakeEnvironment());
    scheduler.start();
    await Promise.resolve();

    void scheduler.requestFollowUp();
    scheduler.stop();
    finish();
    await Promise.resolve();
    await Promise.resolve();

    expect(calls()).toBe(1);
  });

  it('reports a failing run to onError and keeps scheduling', async () => {
    const onError = vi.fn();
    const failure = new Error('offline');
    const run = vi.fn(async () => {
      await Promise.resolve();
      throw failure;
    });
    const scheduler = new SyncScheduler(run, fakeEnvironment(false), { onError });

    await scheduler.trigger();
    await scheduler.trigger();

    expect(onError).toHaveBeenCalledTimes(2);
    expect(onError).toHaveBeenCalledWith(failure);
  });

  it('stop clears the interval and every subscription', async () => {
    const environment = fakeEnvironment();
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment);

    scheduler.start();
    await Promise.resolve();
    scheduler.stop();
    environment.setActive(true);
    environment.goOnline();
    await Promise.resolve();

    expect(environment.intervalCount()).toBe(0);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it('start is idempotent', async () => {
    const environment = fakeEnvironment();
    const run = vi.fn(async () => Promise.resolve());
    const scheduler = new SyncScheduler(run, environment);

    scheduler.start();
    scheduler.start();
    await Promise.resolve();
    await Promise.resolve();

    expect(run).toHaveBeenCalledTimes(1);
    expect(environment.intervalCount()).toBe(1);
  });
});
