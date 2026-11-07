import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildAuthorizationUrl } from '@baukit/auth-web';

const oidc = vi.hoisted(() => {
  const unsubscribe = vi.fn();
  return {
    constructed: [] as unknown[],
    unsubscribe,
    hasSession: vi.fn(() => true),
    login: vi.fn(() => Promise.resolve()),
    handleCallback: vi.fn(() => Promise.resolve(true)),
    accessToken: vi.fn<
      (options?: { readonly forceRefresh?: boolean }) => Promise<string | undefined>
    >(() => Promise.resolve('access-token')),
    subscribeSessionExpired: vi.fn<(listener: unknown) => () => void>(() => unsubscribe),
    logout: vi.fn(() => Promise.resolve(true)),
    clearSession: vi.fn(),
  };
});

const fakeClient = vi.hoisted(
  () => () =>
    class FakeProviderClient {
    public constructor(options: unknown) {
      oidc.constructed.push(options);
    }
    public hasSession = oidc.hasSession;
    public login = oidc.login;
    public handleCallback = oidc.handleCallback;
    public accessToken = oidc.accessToken;
    public subscribeSessionExpired = oidc.subscribeSessionExpired;
    public logout = oidc.logout;
    public clearSession = oidc.clearSession;
    },
);

vi.mock('@baukit/auth-web', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@baukit/auth-web')>()),
  BrowserSessionClient: fakeClient(),
}));
{% if not context.auth_oidc %}vi.mock('{{ "@baukit/auth-web/clerk" if context.auth_clerk else "@baukit/auth-web/workos" }}', () => ({
  {{ "ClerkWebClient" if context.auth_clerk else "WorkOsWebClient" }}: fakeClient(),
}));
{% endif %}
async function loadAuthClient(): Promise<typeof import('./auth').authClient> {
  vi.resetModules();
  return (await import('./auth')).authClient;
}

beforeEach(() => {
  oidc.constructed.length = 0;
  vi.clearAllMocks();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe('OIDC authorization request', () => {
  it('requires authorization code with S256 PKCE', () => {
    const url = buildAuthorizationUrl(
      {
        clientId: 'product-web',
        redirectUri: 'https://app.example.test/',
        scopes: ['profile', 'email'],
        offlineAccess: true,
      },
      'https://login.example.test/authorize',
      { state: 'state-value', challenge: 'challenge-value' },
    );

    expect(url.pathname).toBe('/authorize');
    expect(url.searchParams.get('response_type')).toBe('code');
    expect(url.searchParams.get('code_challenge_method')).toBe('S256');
    expect(url.searchParams.get('code_challenge')).toBe('challenge-value');
    expect(url.searchParams.get('state')).toBe('state-value');
    expect(url.searchParams.get('scope')).toBe('openid profile email offline_access');
  });
});

describe('authClient without a browser window', () => {
  it('reports no session and never builds the OIDC client', async () => {
    const authClient = await loadAuthClient();
    const listener = vi.fn();

    expect(authClient.hasSession()).toBe(false);
    await expect(authClient.handleCallback()).resolves.toBe(false);
    await expect(authClient.accessToken()).resolves.toBeUndefined();
    authClient.subscribeSessionExpired(listener)();
    await expect(authClient.signOut()).resolves.toBe(false);
    await expect(authClient.session()).resolves.toBeUndefined();
    expect(oidc.constructed).toEqual([]);
    expect(listener).not.toHaveBeenCalled();
  });
});

describe('authClient in the browser', () => {
  beforeEach(() => {
    vi.stubGlobal('window', { location: { origin: 'https://app.example.test' } });
  });

{% if context.auth_oidc %}  it('builds one OIDC client from the local development defaults', async () => {
    vi.stubEnv('VITE_OIDC_ISSUER', undefined);
    vi.stubEnv('VITE_OIDC_CLIENT_ID', undefined);
    const authClient = await loadAuthClient();

    expect(authClient.hasSession()).toBe(true);
    expect(authClient.hasSession()).toBe(true);

    expect(oidc.constructed).toHaveLength(1);
    expect(oidc.constructed[0]).toMatchObject({
      apiBaseUrl: expect.stringMatching(/^http:\/\/localhost:\d+$/) as unknown,
      issuer: expect.stringMatching(/^http:\/\/localhost:\d+\/realms\/[^/]+$/) as unknown,
      clientId: expect.stringMatching(/-web$/) as unknown,
      redirectUri: 'https://app.example.test/',
      scopes: ['openid', 'profile', 'email'],
      offlineAccess: true,
      storageKeyPrefix: expect.stringMatching(/:oidc$/) as unknown,
    });
  });

  it('uses the configured issuer and client ID', async () => {
    vi.stubEnv('VITE_OIDC_ISSUER', 'https://login.example.test/realms/product');
    vi.stubEnv('VITE_OIDC_CLIENT_ID', 'product-web');
    vi.stubEnv('VITE_OIDC_AUDIENCE', 'https://api.example.test');
    vi.stubEnv('VITE_OIDC_RESOURCE', 'https://api.example.test');
    vi.stubEnv('VITE_OIDC_SCOPES', 'openid api/read');
    vi.stubEnv('VITE_OIDC_OFFLINE_ACCESS', 'false');
    vi.stubEnv('VITE_API_URL', 'https://api.example.test');
    const authClient = await loadAuthClient();

    authClient.hasSession();

    expect(oidc.constructed[0]).toMatchObject({
      apiBaseUrl: 'https://api.example.test',
      issuer: 'https://login.example.test/realms/product',
      clientId: 'product-web',
      audience: 'https://api.example.test',
      resource: 'https://api.example.test',
      scopes: ['openid', 'api/read'],
      offlineAccess: false,
    });
  });

{% else %}  it('signs in against the local realm outside production builds', async () => {
    const authClient = await loadAuthClient();

    expect(authClient.providerLabel).toBe('local Keycloak');
    authClient.hasSession();
    authClient.hasSession();

    expect(oidc.constructed).toEqual([
      {
        apiBaseUrl: expect.stringMatching(/^http:\/\/localhost:\d+$/) as unknown,
        issuer: expect.stringMatching(/^http:\/\/localhost:\d+\/realms\/[^/]+$/) as unknown,
        clientId: expect.stringMatching(/-web$/) as unknown,
        redirectUri: 'https://app.example.test/',
        scopes: [],
        offlineAccess: false,
        storageKeyPrefix: expect.stringMatching(/:local-oidc$/) as unknown,
      },
    ]);
  });

  it('uses the configured provider SDK once in production builds', async () => {
    vi.stubEnv('MODE', 'production');
    vi.stubEnv('{{ "VITE_CLERK_PUBLISHABLE_KEY" if context.auth_clerk else "VITE_WORKOS_CLIENT_ID" }}', 'public-provider-id');
    const auth = await loadAuthClient();
    auth.hasSession();
    auth.hasSession();
    expect(auth.providerLabel).toBe('{{ "Clerk" if context.auth_clerk else "WorkOS" }}');
    expect(oidc.constructed).toEqual([{ {{ "publishableKey" if context.auth_clerk else "clientId" }}: 'public-provider-id', redirectUri: 'https://app.example.test/' }]);
  });

{% endif %}  it('delegates each call to the OIDC client', async () => {
    const authClient = await loadAuthClient();
    const listener = vi.fn();

    await authClient.signIn();
    await expect(authClient.handleCallback()).resolves.toBe(true);
    await expect(authClient.accessToken({ forceRefresh: true })).resolves.toBe('access-token');
    authClient.subscribeSessionExpired(listener)();
    await expect(authClient.signOut()).resolves.toBe(true);

    expect(oidc.login).toHaveBeenCalledOnce();
    expect(oidc.logout).toHaveBeenCalledOnce();
    expect(oidc.accessToken).toHaveBeenCalledWith({ forceRefresh: true });
    expect(oidc.subscribeSessionExpired).toHaveBeenCalledWith(listener);
    expect(oidc.unsubscribe).toHaveBeenCalledOnce();
  });

  it('reads the session, user and organization from the access token', async () => {
    const payload = btoa(JSON.stringify({ sub: 'user_1', sid: 'sess_1', org_id: 'org_local', role: 'admin', permissions: [] }));
    oidc.accessToken.mockResolvedValueOnce(`header.${payload.replace(/=+$/u, '')}.signature`);
    const authClient = await loadAuthClient();

    await expect(authClient.session()).resolves.toEqual({
      sessionId: 'sess_1',
      user: { id: 'user_1' },
      organization: { id: 'org_local', role: 'admin', permissions: [] },
    });
  });

  it('asks for a cached token by default', async () => {
    const authClient = await loadAuthClient();

    await authClient.accessToken();

    expect(oidc.accessToken).toHaveBeenCalledWith({});
  });
});

it('clears local tokens for profile erasure without a logout redirect', async () => {
  vi.stubGlobal('window', { location: { origin: 'https://app.example.test' } });
  const auth = await loadAuthClient();
  auth.clearSession({ reason: 'profile_erased' });
  expect(oidc.clearSession).toHaveBeenCalledOnce();
  expect(oidc.clearSession).toHaveBeenCalledWith({ reason: 'profile_erased' });
  expect(oidc.logout).not.toHaveBeenCalled();
});
