import { act, renderHook, waitFor } from '@testing-library/react-native';
import type { PropsWithChildren } from 'react';
import { createExpoOidcClient } from '@baukit/auth-native/expo';
import {
  appearanceStateDecoration,
  OidcError,
  type OidcSession,
  type SessionExpiredEvent,
} from '@baukit/auth-native';

jest.mock('expo-auth-session', () => ({ makeRedirectUri: () => 'product://oauth' }));
jest.mock('expo-crypto', () => ({ getRandomBytesAsync: jest.fn() }));
jest.mock('@baukit/auth-native/expo', () => {
  const client = {
    subscribe: jest.fn(),
    subscribeSessionExpired: jest.fn(),
    initialize: jest.fn(),
    accessToken: jest.fn(),
    signIn: jest.fn(),
    signOut: jest.fn(),
  };
  return { completeExpoAuthSession: jest.fn(), createExpoOidcClient: jest.fn(() => client) };
});

import { authClient, OidcAuthProvider, useOidcAuth } from './auth';
import { authStorage } from './auth-storage';

const client = jest.mocked(authClient);
const clientEnvironment = jest.mocked(createExpoOidcClient).mock.calls[0]?.[1];
const REFRESH_LEAD_MS = 30_000;

function session(overrides: Partial<OidcSession> = {}): OidcSession {
  return {
    subject: 'subject-123',
    accessToken: 'access-token',
    expiresAt: Date.now() + 5 * 60_000,
    ...overrides,
  };
}

function wrapper({ children }: PropsWithChildren) {
  return <OidcAuthProvider>{children}</OidcAuthProvider>;
}

async function renderAuth() {
  const rendered = await renderHook(() => useOidcAuth(), { wrapper });
  await waitFor(() => {
    expect(rendered.result.current.ready).toBe(true);
  });
  return rendered;
}

let sessionListener: ((next: OidcSession | undefined) => void) | undefined;
let expiredListener: ((event: SessionExpiredEvent) => void) | undefined;
const unsubscribe = jest.fn();
const unsubscribeExpired = jest.fn();

beforeEach(() => {
  sessionListener = undefined;
  expiredListener = undefined;
  client.subscribe.mockImplementation((listener) => {
    sessionListener = listener;
    return unsubscribe;
  });
  client.subscribeSessionExpired.mockImplementation((listener) => {
    expiredListener = listener;
    return unsubscribeExpired;
  });
  client.initialize.mockResolvedValue(undefined);
  client.accessToken.mockResolvedValue('access-token');
});

afterEach(() => {
  jest.useRealTimers();
});

describe('useOidcAuth', () => {
  it('uses the SecureStore key adapter', () => {
    expect(clientEnvironment?.storage).toBe(authStorage);
  });

  it('requires the provider', async () => {
    jest.spyOn(console, 'error').mockImplementation(() => undefined);

    await expect(renderHook(() => useOidcAuth())).rejects.toThrow(
      'useOidcAuth must be used within OidcAuthProvider.',
    );
  });

  it('exposes the restored session once initialization finishes', async () => {
    client.initialize.mockResolvedValue(session());

    const { result } = await renderAuth();

    expect(result.current).toMatchObject({
      accessToken: 'access-token',
      subject: 'subject-123',
      ready: true,
      sessionExpired: false,
    });
    expect(result.current.error).toBeUndefined();
  });

  it('reports a safe message when initialization fails', async () => {
    client.initialize.mockRejectedValue(new OidcError('discovery_failed'));

    const { result } = await renderAuth();

    expect(result.current.error).toBe('OIDC provider discovery failed.');
    expect(result.current.subject).toBeUndefined();
  });

  it('follows session changes and unsubscribes on unmount', async () => {
    const { result, unmount } = await renderAuth();
    expect(result.current.subject).toBeUndefined();

    await act(() => {
      sessionListener?.(session({ subject: 'subject-456' }));
    });
    expect(result.current.subject).toBe('subject-456');

    await unmount();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
    expect(unsubscribeExpired).toHaveBeenCalledTimes(1);
  });

  it('announces an expired session', async () => {
    const { result } = await renderAuth();

    await act(() => {
      expiredListener?.({ type: 'session-expired', reason: 'refresh_rejected' });
    });

    expect(result.current.sessionExpired).toBe(true);
    expect(result.current.announcement).toBe('Your session expired. Sign in again to continue.');
  });

  it('refreshes the token shortly before it expires and reports a failed refresh', async () => {
    jest.useFakeTimers();
    const expiresInMs = REFRESH_LEAD_MS + 10_000;
    client.initialize.mockResolvedValue(session({ expiresAt: Date.now() + expiresInMs }));
    client.accessToken.mockRejectedValue(new OidcError('refresh_failed', { retryable: true }));
    const { result } = await renderAuth();

    await act(() => jest.advanceTimersByTimeAsync(expiresInMs - REFRESH_LEAD_MS - 1));
    expect(client.accessToken.mock.calls).toHaveLength(0);

    await act(() => jest.advanceTimersByTimeAsync(1));
    expect(client.accessToken.mock.calls).toHaveLength(1);
    expect(result.current.error).toBe('OIDC token refresh failed.');
  });

  it('passes the app appearance to the login page and clears earlier feedback', async () => {
    client.signIn.mockResolvedValue({ status: 'success', subject: 'subject-123' });
    const { result } = await renderAuth();
    await act(() => {
      expiredListener?.({ type: 'session-expired', reason: 'refresh_rejected' });
    });

    const outcome = await act(() => result.current.signIn('dark'));

    expect(outcome).toEqual({ status: 'success', subject: 'subject-123' });
    expect(client.signIn.mock.calls).toEqual([
      [{ stateDecoration: appearanceStateDecoration({ mode: 'dark' }) }],
    ]);
    expect(result.current.sessionExpired).toBe(false);
    expect(result.current.announcement).toBeUndefined();
  });

  it('announces a cancelled sign-in without an appearance', async () => {
    client.signIn.mockResolvedValue({ status: 'cancelled', reason: 'cancel' });
    const { result } = await renderAuth();

    await act(() => result.current.signIn());

    expect(client.signIn.mock.calls).toEqual([[{}]]);
    expect(result.current.announcement).toBe('Sign in cancelled. You can try again.');
  });

  it('reports a failed sign-in', async () => {
    client.signIn.mockRejectedValue(new Error('network down'));
    const { result } = await renderAuth();

    const outcome = await act(() => result.current.signIn());

    expect(outcome).toBeUndefined();
    expect(result.current.error).toBe('OIDC login failed.');
  });

  it('returns the sign-out result and reports a failed sign-out', async () => {
    client.signOut.mockResolvedValueOnce({ providerLogout: 'completed' });
    client.signOut.mockRejectedValueOnce(new Error('network down'));
    const { result } = await renderAuth();

    const completed = await act(() => result.current.signOut());
    const failed = await act(() => result.current.signOut());

    expect(completed).toEqual({ providerLogout: 'completed' });
    expect(failed).toBeUndefined();
    expect(result.current.error).toBe('OIDC login failed.');
  });
});
