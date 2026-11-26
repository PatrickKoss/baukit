import { ApiUnavailableError, NetworkError } from '@baukit/api-runtime';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createApiAvailabilityStore, trackApiAvailability } from './api-availability';

const NOW = 1_700_000_000_000;
const COOLDOWN_MS = 30_000;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('api availability', () => {
  it('opens on a cooldown rejection and clears itself at retryAt', async () => {
    const store = createApiAvailabilityStore();
    const listener = vi.fn();
    store.subscribe(listener);
    const fetch = trackApiAvailability(
      () => Promise.reject(new ApiUnavailableError('req-1', NOW + COOLDOWN_MS)),
      store,
    );

    await expect(fetch('/items')).rejects.toBeInstanceOf(ApiUnavailableError);
    expect(store.retryAt()).toBe(NOW + COOLDOWN_MS);
    expect(listener).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(COOLDOWN_MS - 1);
    expect(store.retryAt()).toBe(NOW + COOLDOWN_MS);
    vi.advanceTimersByTime(1);
    expect(store.retryAt()).toBeUndefined();
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it('closes on any response and ignores other failures', async () => {
    const store = createApiAvailabilityStore();
    store.markUnavailable(NOW + COOLDOWN_MS);
    const failing = trackApiAvailability(
      () => Promise.reject(new NetworkError('offline', 'req-2', undefined)),
      store,
    );
    await expect(failing('/items')).rejects.toBeInstanceOf(NetworkError);
    expect(store.retryAt()).toBe(NOW + COOLDOWN_MS);

    const answering = trackApiAvailability(() => Promise.resolve(new Response(null, { status: 500 })), store);
    await answering('/items');
    expect(store.retryAt()).toBeUndefined();
  });

  it('keeps the later retryAt timer when the cooldown is extended', () => {
    const store = createApiAvailabilityStore();
    store.markUnavailable(NOW + COOLDOWN_MS);
    store.markUnavailable(NOW + 2 * COOLDOWN_MS);

    vi.advanceTimersByTime(COOLDOWN_MS);
    expect(store.retryAt()).toBe(NOW + 2 * COOLDOWN_MS);
    vi.advanceTimersByTime(COOLDOWN_MS);
    expect(store.retryAt()).toBeUndefined();
  });

  it('stops notifying after unsubscribe', () => {
    const store = createApiAvailabilityStore();
    const listener = vi.fn();
    store.subscribe(listener)();
    store.markUnavailable(NOW + COOLDOWN_MS);
    expect(listener).not.toHaveBeenCalled();
  });
});
