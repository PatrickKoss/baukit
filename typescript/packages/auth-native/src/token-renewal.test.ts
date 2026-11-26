import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { OidcError } from './oidc-errors.js';
import { scheduleTokenRenewal } from './token-renewal.js';

const NOW = 1_000_000;
const EXPIRES_AT = NOW + 120_000;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('scheduleTokenRenewal', () => {
  it('renews 30 s before expiry and stops after success', async () => {
    const renew = vi.fn().mockResolvedValue('token');
    scheduleTokenRenewal({ expiresAt: EXPIRES_AT, renew });

    await vi.advanceTimersByTimeAsync(89_999);
    expect(renew).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(renew).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(renew).toHaveBeenCalledTimes(1);
  });

  it('reschedules with jittered exponential backoff on a retryable failure', async () => {
    const transient = new OidcError('refresh_failed', { retryable: true, status: 503 });
    const renew = vi
      .fn()
      .mockRejectedValueOnce(transient)
      .mockRejectedValueOnce(new TypeError('Network request failed'))
      .mockResolvedValue('token');
    const onError = vi.fn();
    scheduleTokenRenewal({ expiresAt: NOW, renew, onError, random: () => 0.5 });

    await vi.advanceTimersByTimeAsync(0);
    expect(renew).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(499);
    expect(renew).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(renew).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(999);
    expect(renew).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(renew).toHaveBeenCalledTimes(3);
    expect(onError).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(renew).toHaveBeenCalledTimes(3);
  });

  it('caps the backoff ceiling at maxDelayMs', async () => {
    const renew = vi.fn().mockRejectedValue(new OidcError('refresh_failed', { retryable: true }));
    scheduleTokenRenewal({ expiresAt: NOW, renew, maxDelayMs: 4_000, random: () => 0.999 });

    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(999 + 1_998 + 3_996);
    expect(renew).toHaveBeenCalledTimes(4);
    await vi.advanceTimersByTimeAsync(3_995);
    expect(renew).toHaveBeenCalledTimes(4);
    await vi.advanceTimersByTimeAsync(1);
    expect(renew).toHaveBeenCalledTimes(5);
  });

  it('stops on a terminal refresh rejection', async () => {
    const renew = vi.fn().mockRejectedValue(new OidcError('refresh_failed', { status: 400 }));
    const onError = vi.fn();
    scheduleTokenRenewal({ expiresAt: NOW, renew, onError, random: () => 0 });

    await vi.advanceTimersByTimeAsync(600_000);
    expect(renew).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it('does not retry after cancel', async () => {
    let reject: ((cause: unknown) => void) | undefined;
    const renew = vi.fn(
      () =>
        new Promise((_resolve, rejectRenewal) => {
          reject = rejectRenewal;
        }),
    );
    const onError = vi.fn();
    const cancel = scheduleTokenRenewal({ expiresAt: NOW, renew, onError });

    await vi.advanceTimersByTimeAsync(0);
    cancel();
    reject?.(new OidcError('refresh_failed', { retryable: true }));
    await vi.advanceTimersByTimeAsync(600_000);
    expect(renew).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
  });

  it('clears the pending first attempt on cancel', async () => {
    const renew = vi.fn().mockResolvedValue('token');
    const cancel = scheduleTokenRenewal({ expiresAt: EXPIRES_AT, renew });

    cancel();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(renew).not.toHaveBeenCalled();
  });
});
