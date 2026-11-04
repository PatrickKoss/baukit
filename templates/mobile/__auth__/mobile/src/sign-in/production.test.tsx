import { describe, expect, it } from '@jest/globals';
{% if context.auth_clerk %}import { render, screen, waitFor } from '@testing-library/react-native';
import type { PropsWithChildren } from 'react';
import { Text } from 'react-native';
{% endif %}
import { unverifiedSignInSessionFromJwt } from '@baukit/api-runtime';

jest.mock('expo-constants', () => ({
  __esModule: true,
  default: {
    expoConfig: {
      extra: {% if context.auth_clerk %}{ clerkPublishableKey: 'pk_test_placeholder' }{% else %}{ oidcIssuer: 'https://auth.example.invalid/', oidcClientId: 'client_placeholder' }{% endif %},
    },
  },
}));
jest.mock('../auth-storage', () => ({
  authStorage: {
    get: () => Promise.resolve(null),
    set: () => Promise.resolve(),
    delete: () => Promise.resolve(),
  },
}));

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `header.${encoded.replace(/=+$/u, '')}.signature`;
}

{% if context.auth_clerk %}const mockSdk = {
  getToken: jest.fn<Promise<string | null>, [unknown]>(),
  startHostedAuth: jest.fn<Promise<{ createdSessionId: string | null }>, []>(),
  signOut: jest.fn<Promise<void>, []>(() => Promise.resolve()),
  providerProps: [] as unknown[],
};

jest.mock('@clerk/expo', () => ({
  ClerkProvider: (props: PropsWithChildren<Record<string, unknown>>) => {
    mockSdk.providerProps.push(props);
    return props.children;
  },
  useAuth: () => ({ isLoaded: true }),
  useClerk: () => ({
    user: { id: 'user_clerk' },
    session: { getToken: (options: unknown) => mockSdk.getToken(options) },
    signOut: () => mockSdk.signOut(),
  }),
}));
jest.mock('@clerk/expo/hosted-auth', () => ({
  useHostedAuth: () => ({ startHostedAuth: () => mockSdk.startHostedAuth() }),
}));

import { signInAdapter } from './production';

const clerkToken = unsignedJwt({
  sub: 'user_clerk',
  sid: 'sess_clerk',
  v: 2,
  o: { id: 'org_clerk', rol: 'admin', slg: 'acme', per: 'items:read,items:write' },
  exp: Math.floor(Date.now() / 1000) + 300,
});

async function mountProvider(): Promise<void> {
  const Provider = signInAdapter.Provider;
  if (Provider === undefined) throw new Error('Clerk adapter must provide its SDK provider.');
  await render(
    <Provider>
      <Text>app</Text>
    </Provider>,
  );
  await waitFor(() => {
    expect(screen.getByText('app')).toBeTruthy();
  });
}

describe('Clerk production sign-in adapter', () => {
  it('hands the publishable key to the Clerk SDK and maps its session token', async () => {
    mockSdk.getToken.mockResolvedValue(clerkToken);
    mockSdk.startHostedAuth.mockResolvedValue({ createdSessionId: 'sess_clerk' });
    await mountProvider();

    expect(signInAdapter.providerLabel).toBe('Clerk');
    expect(mockSdk.providerProps[0]).toMatchObject({ publishableKey: 'pk_test_placeholder' });
    await expect(signInAdapter.client.signIn()).resolves.toMatchObject({ status: 'success' });
    const token = await signInAdapter.client.accessToken();
    expect(unverifiedSignInSessionFromJwt(token ?? '')).toEqual({
      sessionId: 'sess_clerk',
      user: { id: 'user_clerk' },
      organization: { id: 'org_clerk', role: 'admin', slug: 'acme', permissions: ['items:read', 'items:write'] },
    });

    await signInAdapter.client.signOut();
    expect(mockSdk.signOut).toHaveBeenCalledTimes(1);
  });

  it('reports a cancelled hosted sign-in', async () => {
    mockSdk.startHostedAuth.mockResolvedValue({ createdSessionId: null });
    await mountProvider();

    await expect(signInAdapter.client.signIn()).resolves.toMatchObject({ status: 'cancelled' });
  });
});
{% else %}const mockBrowser = {
  prompt: jest.fn<Promise<unknown>, []>(),
  requests: [] as unknown[],
};

jest.mock('expo-crypto', () => ({ getRandomBytesAsync: (size: number) => Promise.resolve(new Uint8Array(size)) }));
jest.mock('expo-web-browser', () => ({
  maybeCompleteAuthSession: jest.fn(),
  openAuthSessionAsync: jest.fn(() => Promise.resolve({ type: 'success' })),
}));
jest.mock('expo-auth-session', () => ({
  makeRedirectUri: ({ scheme, path }: { scheme: string; path: string }) => `${scheme}://${path}`,
  ResponseType: { Code: 'code' },
  Prompt: { Login: 'login' },
  AuthRequest: class {
    public readonly state = 'expected-state';
    public readonly codeVerifier = 'verifier-from-auth-session';
    public constructor(request: unknown) {
      mockBrowser.requests.push(request);
    }
    public promptAsync(): Promise<unknown> {
      return mockBrowser.prompt();
    }
  },
}));

const mockFetch = jest.fn<Promise<Response>, [string, RequestInit | undefined]>();

jest.mock('@baukit/auth-native/expo', () => {
  const actual = jest.requireActual<typeof import('@baukit/auth-native/expo')>('@baukit/auth-native/expo');
  return {
    ...actual,
    createExpoOidcEnvironment: (...[options]: Parameters<typeof actual.createExpoOidcEnvironment>) =>
      actual.createExpoOidcEnvironment({ ...options, fetch: (url, init) => mockFetch(url, init) }),
  };
});

import { signInAdapter } from './production';

const workosToken = unsignedJwt({
  sub: 'user_workos',
  sid: 'session_workos',
  org_id: 'org_workos',
  role: 'member',
  permissions: ['items:read'],
  exp: Math.floor(Date.now() / 1000) + 300,
});

describe('WorkOS production sign-in adapter', () => {
  it('runs AuthKit PKCE against the configured issuer and maps its access token', async () => {
    const calls: { url: string; body?: string }[] = [];
    mockFetch.mockImplementation((url, init) => {
      calls.push({ url, ...(typeof init?.body === 'string' ? { body: init.body } : {}) });
      return Promise.resolve(
        new Response(JSON.stringify({ access_token: workosToken, refresh_token: 'refresh' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    });
    mockBrowser.prompt.mockResolvedValue({ type: 'success', params: { code: 'workos-code', state: 'expected-state' } });

    expect(signInAdapter.providerLabel).toBe('WorkOS');
    await expect(signInAdapter.client.signIn()).resolves.toEqual({ status: 'success', subject: 'user_workos' });

    expect(mockBrowser.requests[0]).toMatchObject({ clientId: 'client_placeholder', usePKCE: true });
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe('https://auth.example.invalid/user_management/authenticate');
    expect(JSON.parse(calls[0]?.body ?? '{}')).toMatchObject({
      grant_type: 'authorization_code',
      code: 'workos-code',
      code_verifier: 'verifier-from-auth-session',
    });
    const token = await signInAdapter.client.accessToken();
    expect(unverifiedSignInSessionFromJwt(token ?? '')).toEqual({
      sessionId: 'session_workos',
      user: { id: 'user_workos' },
      organization: { id: 'org_workos', role: 'member', permissions: ['items:read'] },
    });
  });

  it('reports a dismissed browser sign-in', async () => {
    mockBrowser.prompt.mockResolvedValue({ type: 'dismiss' });

    await expect(signInAdapter.client.signIn()).resolves.toEqual({ status: 'cancelled', reason: 'dismiss' });
  });
});
{% endif %}