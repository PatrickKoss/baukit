// @vitest-environment jsdom

import { QueryClient } from '@tanstack/react-query';
import { PersistenceIdentityMismatchError } from '@baukit/data-contracts';
import type { SessionExpiredReason } from '@baukit/auth-web';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAuthenticatedLocalData } from './local-data';

interface HookProps {
  readonly profileId: string | undefined;
  readonly sessionExpiredReason: SessionExpiredReason | undefined;
}

function renderLocalData(initialProps: HookProps, queryClient = new QueryClient()) {
  return renderHook(
    ({ profileId, sessionExpiredReason }: HookProps) =>
      useAuthenticatedLocalData(profileId, sessionExpiredReason, queryClient),
    { initialProps },
  );
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('useAuthenticatedLocalData', () => {
  it('opens a partition for the server profile and records ownership', async () => {
    const queryClient = new QueryClient();
    const clear = vi.spyOn(queryClient, 'clear');
    const { result } = renderLocalData(
      { profileId: 'account-a', sessionExpiredReason: undefined },
      queryClient,
    );

    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });
    expect(result.current.state).toMatchObject({ partition: { subject: 'account-a' } });
    expect(clear).toHaveBeenCalledOnce();
    expect(Object.keys(localStorage)).toContainEqual(
      expect.stringMatching(/:local-data-registry:v1$/),
    );
  });

  it('closes the partition and resets caches on sign-out', async () => {
    const queryClient = new QueryClient();
    const clear = vi.spyOn(queryClient, 'clear');
    const { result, rerender } = renderLocalData(
      { profileId: 'account-a', sessionExpiredReason: undefined },
      queryClient,
    );
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    rerender({ profileId: undefined, sessionExpiredReason: undefined });

    await waitFor(() => {
      expect(result.current.state.status).toBe('signed-out');
    });
    expect(clear).toHaveBeenCalledTimes(2);
  });

  it('blocks on terminal session expiry', async () => {
    const { result, rerender } = renderLocalData({ profileId: 'account-a', sessionExpiredReason: undefined });
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    rerender({ profileId: 'account-a', sessionExpiredReason: 'refresh_rejected' });

    await waitFor(() => {
      expect(result.current.state).toMatchObject({
        status: 'blocked',
        reason: 'session-expired',
      });
    });
  });

  it('clears on request', async () => {
    const { result } = renderLocalData({ profileId: 'account-a', sessionExpiredReason: undefined });
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    await act(() => result.current.clear());

    expect(result.current.state.status).toBe('signed-out');
  });

  it.each([false, true, 'refresh_rejected'] as const)('erases on profile expiry after token clearing: %s', async (closed) => {
    const queryClient = new QueryClient();
    const { result, rerender } = renderLocalData(
      { profileId: 'erased-profile', sessionExpiredReason: undefined }, queryClient,
    );
    await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
    queryClient.setQueryData(['private'], 'erased data');
    if (closed) {
      rerender({ profileId: undefined, sessionExpiredReason: closed === 'refresh_rejected' ? closed : undefined });
      await waitFor(() => {
        expect(result.current.state.status).toBe(closed === 'refresh_rejected' ? 'blocked' : 'signed-out');
      });
    }

    rerender({ profileId: undefined, sessionExpiredReason: 'profile_erased' });

    await waitFor(() => {
      expect(result.current.state).toMatchObject({ status: 'blocked', reason: 'profile-erased', cleanup: 'complete' });
    });
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
    expect(localStorage.getItem(registry ?? '')).not.toContain('erased-profile');
  });

  it('keeps expiry cleanup stable when profile confirmation clears, then opens a fresh profile', async () => {
    const queryClient = new QueryClient();
    const clear = vi.spyOn(queryClient, 'clear');
    const { result, rerender } = renderLocalData(
      { profileId: 'erased-profile', sessionExpiredReason: undefined }, queryClient,
    );
    await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
    const previous = result.current.state;
    if (previous.status !== 'ready') throw new Error('Expected a ready partition.');

    rerender({ profileId: 'erased-profile', sessionExpiredReason: 'profile_erased' });
    await waitFor(() => {
      expect(result.current.state).toMatchObject({ status: 'blocked', reason: 'profile-erased', cleanup: 'complete' });
    });
    clear.mockClear();
    await act(async () => {
      rerender({ profileId: undefined, sessionExpiredReason: 'profile_erased' });
      await Promise.resolve();
    });
    expect(clear).not.toHaveBeenCalled();

    rerender({ profileId: 'new-profile', sessionExpiredReason: undefined });
    await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
    const next = result.current.state;
    if (next.status !== 'ready') throw new Error('Expected a ready partition.');
    expect(next.partition.subject).toBe('new-profile');
    expect(next.partition.storeName).not.toBe(previous.partition.storeName);
    const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
    expect(localStorage.getItem(registry ?? '')).not.toContain('erased-profile');
    expect(localStorage.getItem(registry ?? '')).toContain('new-profile');
  });

  it('opens a fresh partition after re-onboarding without an expiry event', async () => {
    const first = renderLocalData({ profileId: 'old-profile', sessionExpiredReason: undefined });
    await waitFor(() => { expect(first.result.current.state.status).toBe('ready'); });
    const oldState = first.result.current.state;
    if (oldState.status !== 'ready') throw new Error('Expected a ready partition.');
    first.rerender({ profileId: undefined, sessionExpiredReason: 'refresh_rejected' });
    await waitFor(() => { expect(first.result.current.state.status).toBe('blocked'); });
    first.unmount();

    const next = renderLocalData({ profileId: 'new-profile', sessionExpiredReason: undefined });
    await waitFor(() => { expect(next.result.current.state.status).toBe('ready'); });
    const newState = next.result.current.state;
    if (newState.status !== 'ready') throw new Error('Expected a ready partition.');
    expect(newState.partition.storeName).not.toBe(oldState.partition.storeName);
    const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
    expect(localStorage.getItem(registry ?? '')).toContain('old-profile');
    expect(localStorage.getItem(registry ?? '')).toContain('new-profile');
  });

  it('erases the account registry and cached data', async () => {
    const queryClient = new QueryClient();
    const { result } = renderLocalData({ profileId: 'account-erase', sessionExpiredReason: undefined }, queryClient);
    await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
    queryClient.setQueryData(['private'], 'private content');
    await act(() => result.current.erase('account-erase'));
    expect(result.current.state.status).toBe('signed-out');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
    expect(registry).toBeDefined();
    expect(localStorage.getItem(registry ?? '')).not.toContain('account-erase');
  });

  it('blocks on an identity mismatch reported by the API layer', async () => {
    const { result } = renderLocalData({ profileId: 'account-a', sessionExpiredReason: undefined });
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    await act(() => result.current.blockIdentityMismatch(new PersistenceIdentityMismatchError()));

    expect(result.current.state).toMatchObject({
      status: 'blocked',
      reason: 'identity-mismatch',
    });
  });

  it('blocks instead of opening when the ownership registry is corrupt', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockReturnValue('not json');
    const { result } = renderLocalData({ profileId: 'account-a', sessionExpiredReason: undefined });

    await waitFor(() => {
      expect(result.current.state).toMatchObject({
        status: 'blocked',
        reason: 'identity-mismatch',
      });
    });
  });

  it('ignores a transition that settles after unmount', async () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    const { result, unmount } = renderLocalData({
      profileId: 'account-a',
      sessionExpiredReason: undefined,
    });

    unmount();

    await waitFor(() => {
      expect(setItem).toHaveBeenCalled();
    });
    expect(result.current.state.status).toBe('signed-out');
  });
});


it('retains a different account partition if the subject changes during erasure', async () => {
  const { result } = renderLocalData({ profileId: 'account-b', sessionExpiredReason: undefined });
  await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
  await expect(result.current.erase('account-a')).rejects.toThrow(PersistenceIdentityMismatchError);
  expect(result.current.state).toMatchObject({ status: 'ready', partition: { subject: 'account-b' } });
});
