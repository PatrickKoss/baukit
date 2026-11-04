import { act, renderHook, waitFor } from '@testing-library/react-native';
import type { PropsWithChildren } from 'react';
{% if context.auth_oidc %}import { createExpoOidcClient as createClient } from '@baukit/auth-native/expo';
{% endif %}import {
  appearanceStateDecoration,
  OidcError,
  type OidcSession,
  type SessionExpiredEvent,
} from '@baukit/auth-native';

{% if context.auth_oidc %}jest.mock('expo-auth-session', () => ({ makeRedirectUri: () => 'product://oauth' }));
jest.mock('expo-crypto', () => ({ getRandomBytesAsync: jest.fn() }));
jest.mock('expo-constants', () => ({
  __esModule: true,
  default: { expoConfig: { extra: {
    oidcAudience: 'https://api.example.test',
    oidcResource: 'https://api.example.test',
    oidcScopes: 'openid api/read',
    oidcOfflineAccess: false,
  } } },
}));
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
{% else %}jest.mock('./sign-in/local', () => ({
  signInAdapter: {
    providerLabel: 'local Keycloak',
    client: {
      subscribe: jest.fn(),
      subscribeSessionExpired: jest.fn(),
      initialize: jest.fn(),
      accessToken: jest.fn(),
      signIn: jest.fn(),
      signOut: jest.fn(),
    },
    Provider: ({ children }: PropsWithChildren) => children,
  },
}));
{% endif %}
import { authClient, AuthProvider, useAuth } from './auth';
{% if context.auth_oidc %}import { authStorage } from './auth-storage';
{% endif %}
const client = jest.mocked(authClient);
{% if context.auth_oidc %}const clientEnvironment = jest.mocked(createClient).mock.calls[0]?.[1];
const clientConfiguration = jest.mocked(createClient).mock.calls[0]?.[0];
{% endif %}const REFRESH_LEAD_MS = 30_000;

function session(overrides: Partial<OidcSession> = {}): OidcSession {
  return {
    subject: 'subject-123',
    accessToken: 'access-token',
    issuedAt: Date.now(),
    expiresAt: Date.now() + 5 * 60_000,
    ...overrides,
  };
}

function wrapper({ children }: PropsWithChildren) {
  return <AuthProvider>{children}</AuthProvider>;
}

async function renderAuth() {
  const rendered = await renderHook(() => useAuth(), { wrapper });
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

describe('useAuth', () => {
{% if context.auth_oidc %}  it('passes external API token binding and scopes from Expo configuration', () => {
    expect(clientConfiguration).toMatchObject({
      audience: 'https://api.example.test',
      resource: 'https://api.example.test',
      scopes: ['openid', 'api/read'],
      offlineAccess: false,
    });
  });

  it('uses the SecureStore storage port', () => {
    expect(clientEnvironment?.storage).toBe(authStorage);
  });

{% endif %}
  it('requires the provider', async () => {
    jest.spyOn(console, 'error').mockImplementation(() => undefined);

    await expect(renderHook(() => useAuth())).rejects.toThrow(
      'useAuth must be used within AuthProvider.',
    );
  });

  it('exposes the restored session once initialization finishes', async () => {
    client.initialize.mockResolvedValue(session());

    const { result } = await renderAuth();

    expect(result.current).toMatchObject({
      providerLabel: 'local Keycloak',
      accessToken: 'access-token',
      subject: 'subject-123',
      ready: true,
      sessionExpired: false,
    });
    expect(result.current.error).toBeUndefined();
  });

  it('reads the user and organization from the access token', async () => {
    const claims = btoa(JSON.stringify({ sub: 'subject-123', sid: 'sess_1', org_id: 'org_local', role: 'admin', permissions: ['items:read'] }));
    client.initialize.mockResolvedValue(session({ accessToken: `header.${claims.replace(/=+$/u, '')}.signature` }));

    const { result } = await renderAuth();

    expect(result.current.session).toEqual({
      sessionId: 'sess_1',
      user: { id: 'subject-123' },
      organization: { id: 'org_local', role: 'admin', permissions: ['items:read'] },
    });
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

  it.each(['refresh_rejected', 'refresh_unavailable', 'profile_erased'] as const)('announces an expired session for %s', async (reason) => {
    const { result } = await renderAuth();

    await act(() => {
      expiredListener?.({ type: 'session-expired', reason });
    });

    expect(result.current.sessionExpired).toBe(true);
    expect(result.current.sessionExpiredReason).toBe(reason);
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
    expect(result.current.sessionExpiredReason).toBeUndefined();
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
