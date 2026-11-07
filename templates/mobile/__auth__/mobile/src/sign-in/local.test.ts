import { describe, expect, it } from '@jest/globals';

const mockBrowser = {
  prompt: jest.fn(),
  requests: [] as unknown[],
};

jest.mock('expo-constants', () => ({ __esModule: true, default: { expoConfig: { extra: {} } } }));
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
    public promptAsync(discovery: unknown): Promise<unknown> {
      return mockBrowser.prompt(discovery) as Promise<unknown>;
    }
  },
}));
jest.mock('../auth-storage', () => {
  const values = new Map<string, string>();
  return {
    authStorage: {
      get: (key: string) => Promise.resolve(values.get(key) ?? null),
      set: (key: string, value: string) => {
        values.set(key, value);
        return Promise.resolve();
      },
      delete: (key: string) => {
        values.delete(key);
        return Promise.resolve();
      },
    },
  };
});

const mockFetch = jest.fn<Promise<Response>, [string, RequestInit | undefined]>();

function requestBody(init: RequestInit | undefined): string | undefined {
  if (init?.body instanceof URLSearchParams) return init.body.toString();
  return typeof init?.body === 'string' ? init.body : undefined;
}

jest.mock('@baukit/auth-native/expo', () => {
  const actual = jest.requireActual<typeof import('@baukit/auth-native/expo')>('@baukit/auth-native/expo');
  return {
    ...actual,
    createExpoOidcClient: (...[config, options]: Parameters<typeof actual.createExpoOidcClient>) =>
      actual.createExpoOidcClient(config, { ...options, fetch: (url, init) => mockFetch(url, init) }),
  };
});

import { unverifiedSignInSessionFromJwt } from '@baukit/api-runtime';

import { signInAdapter } from './local';

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `header.${encoded.replace(/=+$/u, '')}.signature`;
}

const accessToken = unsignedJwt({
{% if context.auth_clerk %}  sub: 'local-user',
  sid: 'local-session',
  v: 2,
  o: { id: 'org_local', rol: 'admin', slg: 'local' },
{% else %}  sub: 'local-user',
  sid: 'local-session',
  client_id: 'client_local',
  org_id: 'org_local',
  role: 'admin',
  permissions: ['items:read', 'items:write'],
{% endif %}  exp: Math.floor(Date.now() / 1000) + 300,
});

function json(body: unknown): Response {
  const response: Pick<Response, 'ok' | 'status' | 'json'> = { ok: true, status: 200, json: () => Promise.resolve(body) };
  return response as Response;
}

describe('local Keycloak sign-in adapter', () => {
  it('signs in with PKCE in the system browser and exposes the realm claims', async () => {
    const calls: { url: string; body: string | undefined }[] = [];
    mockFetch.mockImplementation((url, init) => {
      calls.push({ url, body: requestBody(init) });
      if (url.endsWith('/.well-known/openid-configuration')) {
        const issuer = url.replace('/.well-known/openid-configuration', '');
        return Promise.resolve(
          json({
            issuer,
            authorization_endpoint: `${issuer}/protocol/openid-connect/auth`,
            token_endpoint: `${issuer}/protocol/openid-connect/token`,
            userinfo_endpoint: `${issuer}/protocol/openid-connect/userinfo`,
          }),
        );
      }
      if (url.endsWith('/userinfo')) return Promise.resolve(json({ sub: 'local-user' }));
      return Promise.resolve(json({ access_token: accessToken, expires_in: 300, refresh_token: 'refresh' }));
    });
    mockBrowser.prompt.mockResolvedValue({ type: 'success', params: { code: 'local-code', state: 'expected-state' } });

    expect(signInAdapter.providerLabel).toBe('local Keycloak');
    await expect(signInAdapter.client.signIn()).resolves.toEqual({ status: 'success', subject: 'local-user' });

    expect(calls[0]?.url).toMatch(/^http:\/\/localhost:\d+\/realms\/[^/]+\/\.well-known\/openid-configuration$/u);
    expect(mockBrowser.requests[0]).toMatchObject({
      clientId: expect.stringMatching(/-mobile$/u),
      redirectUri: expect.stringMatching(/:\/\/oauth$/u),
      usePKCE: true,
      scopes: ['openid'],
    });
    const token = new URLSearchParams(calls.find(({ url }) => url.endsWith('/token'))?.body);
    expect(token.get('grant_type')).toBe('authorization_code');
    expect(token.get('code_verifier')).toBe('verifier-from-auth-session');

    const current = await signInAdapter.client.accessToken();
    expect(unverifiedSignInSessionFromJwt(current ?? '')).toEqual({
      sessionId: 'local-session',
      user: { id: 'local-user' },
{% if context.auth_clerk %}      organization: { id: 'org_local', role: 'admin', slug: 'local', permissions: [] },
{% else %}      organization: { id: 'org_local', role: 'admin', permissions: ['items:read', 'items:write'] },
{% endif %}    });
  });

  it('reports a cancelled browser sign-in', async () => {
    mockBrowser.prompt.mockResolvedValue({ type: 'cancel' });

    await expect(signInAdapter.client.signIn()).resolves.toEqual({ status: 'cancelled', reason: 'cancel' });
  });
});
