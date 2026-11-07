import { describe, expect, it, vi } from 'vitest';

import { createOpaqueIntervalTimers } from './timer-handles.js';

describe('createOpaqueIntervalTimers', () => {
  it('returns an object handle and clears the matching host id', () => {
    const host = { setInterval: vi.fn(() => 42), clearInterval: vi.fn() };
    const timers = createOpaqueIntervalTimers(host);
    const callback = vi.fn();

    const handle = timers.setInterval(callback, 500);
    timers.clearInterval(handle);

    expect(typeof handle).toBe('object');
    expect(host.setInterval).toHaveBeenCalledWith(callback, 500);
    expect(host.clearInterval).toHaveBeenCalledWith(42);
  });

  it('keeps distinct handles for repeated host ids and ignores unknown or cleared handles', () => {
    const host = { setInterval: vi.fn(() => 0), clearInterval: vi.fn() };
    const timers = createOpaqueIntervalTimers(host);

    const first = timers.setInterval(vi.fn(), 1);
    const second = timers.setInterval(vi.fn(), 1);
    timers.clearInterval(first);
    timers.clearInterval(first);
    timers.clearInterval({});

    expect(first).not.toBe(second);
    expect(host.clearInterval).toHaveBeenCalledTimes(1);
    expect(host.clearInterval).toHaveBeenCalledWith(0);
  });
});
