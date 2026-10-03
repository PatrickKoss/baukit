// @vitest-environment jsdom

import { QueryClient } from '@tanstack/react-query';
import { PersistenceIdentityMismatchError } from '@baukit/data-contracts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAuthenticatedLocalData } from './local-data';

interface HookProps {
  readonly subject: string | undefined;
  readonly sessionExpired: boolean;
}

function renderLocalData(initialProps: HookProps, queryClient = new QueryClient()) {
  return renderHook(
    ({ subject, sessionExpired }: HookProps) =>
      useAuthenticatedLocalData(subject, sessionExpired, queryClient),
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
  it('opens a partition for the signed-in subject and records ownership', async () => {
    const queryClient = new QueryClient();
    const clear = vi.spyOn(queryClient, 'clear');
    const { result } = renderLocalData(
      { subject: 'account-a', sessionExpired: false },
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
      { subject: 'account-a', sessionExpired: false },
      queryClient,
    );
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    rerender({ subject: undefined, sessionExpired: false });

    await waitFor(() => {
      expect(result.current.state.status).toBe('signed-out');
    });
    expect(clear).toHaveBeenCalledTimes(2);
  });

  it('blocks on terminal session expiry', async () => {
    const { result, rerender } = renderLocalData({ subject: 'account-a', sessionExpired: false });
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    rerender({ subject: 'account-a', sessionExpired: true });

    await waitFor(() => {
      expect(result.current.state).toMatchObject({
        status: 'blocked',
        reason: 'session-expired',
      });
    });
  });

  it('clears on request', async () => {
    const { result } = renderLocalData({ subject: 'account-a', sessionExpired: false });
    await waitFor(() => {
      expect(result.current.state.status).toBe('ready');
    });

    await act(() => result.current.clear());

    expect(result.current.state.status).toBe('signed-out');
  });

  it('erases the account registry and cached data', async () => {
    const queryClient = new QueryClient();
    const { result } = renderLocalData({ subject: 'account-erase', sessionExpired: false }, queryClient);
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
    const { result } = renderLocalData({ subject: 'account-a', sessionExpired: false });
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
    const { result } = renderLocalData({ subject: 'account-a', sessionExpired: false });

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
      subject: 'account-a',
      sessionExpired: false,
    });

    unmount();

    await waitFor(() => {
      expect(setItem).toHaveBeenCalled();
    });
    expect(result.current.state.status).toBe('signed-out');
  });
});


it('retains a different account partition if the subject changes during erasure', async () => {
  const { result } = renderLocalData({ subject: 'account-b', sessionExpired: false });
  await waitFor(() => { expect(result.current.state.status).toBe('ready'); });
  await expect(result.current.erase('account-a')).rejects.toThrow(PersistenceIdentityMismatchError);
  expect(result.current.state).toMatchObject({ status: 'ready', partition: { subject: 'account-b' } });
});
