import { describe, expect, it, vi } from 'vitest';

import {
  NativeOidcClient,
  OidcError,
  safeAuthErrorMessage,
  type AuthorizationRequest,
  type AuthorizationResult,
  type BrowserFlowPort,
  type EndSessionRequest,
  type FetchPort,
  type NativeOidcEnvironment,
  type SecureStoragePort,
} from './index.js';

class MemoryStorage implements SecureStoragePort {
  public readonly values = new Map<string, string>();
  public readonly deletes: string[] = [];

  public get(key: string): Promise<string | null> {
    return Promise.resolve(this.values.get(key) ?? null);
  }

  public set(key: string, value: string): Promise<void> {
    this.values.set(key, value);
    return Promise.resolve();
  }

  public delete(key: string): Promise<void> {
    this.deletes.push(key);
    this.values.delete(key);
    return Promise.resolve();
  }
}

class FakeBrowser implements BrowserFlowPort {
  public readonly authorizationRequests: AuthorizationRequest[] = [];
  public readonly endSessionRequests: EndSessionRequest[] = [];
  public authorizationResults: AuthorizationResult[] = [successfulAuthorization()];
  public endSessionResult = true;
  public authorizationError: Error | undefined;
  public endSessionError: Error | undefined;
  public beforeEndSession: (() => void) | undefined;

  public authorize(request: AuthorizationRequest): Promise<AuthorizationResult> {
    this.authorizationRequests.push(request);
    if (this.authorizationError !== undefined) {
      return Promise.reject(this.authorizationError);
    }
    return Promise.resolve(this.authorizationResults.shift() ?? successfulAuthorization());
  }

  public endSession(request: EndSessionRequest): Promise<boolean> {
    this.beforeEndSession?.();
    this.endSessionRequests.push(request);
    if (this.endSessionError !== undefined) {
      return Promise.reject(this.endSessionError);
    }
    return Promise.resolve(this.endSessionResult);
  }
}

interface Harness {
  readonly client: NativeOidcClient;
  readonly storage: MemoryStorage;
  readonly browser: FakeBrowser;
  readonly requests: { readonly url: string; readonly init: RequestInit | undefined }[];
  readonly tokenResponses: unknown[];
  setNow(value: number): void;
}

interface HarnessOptions {
  readonly endSessionEndpoint?: string | null;
  readonly tokenResponses?: unknown[];
  readonly userInfo?: unknown;
  readonly fetchOverride?: FetchPort;
  readonly userInfoNow?: number;
}

const issuer = 'https://identity.example.test/tenant';
const sessionKey = 'native-test:session';
const forceLoginKey = 'native-test:force-login';

function makeHarness(options: HarnessOptions = {}): Harness {
  const storage = new MemoryStorage();
  const browser = new FakeBrowser();
  const requests: { readonly url: string; readonly init: RequestInit | undefined }[] = [];
  const tokenResponses = options.tokenResponses ?? [defaultTokens()];
  let now = 1_000;
  const fetchImplementation: FetchPort =
    options.fetchOverride ??
    ((url, init) => {
      requests.push({ url, init });
      if (url.endsWith('/.well-known/openid-configuration')) {
        const configuredEndSession =
          options.endSessionEndpoint === undefined
            ? 'https://login.example.test/session/end'
            : options.endSessionEndpoint;
        return Promise.resolve(
          jsonResponse({
            issuer,
            authorization_endpoint: 'https://login.example.test/oauth/authorize',
            token_endpoint: 'https://login.example.test/oauth/token',
            userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
            ...(configuredEndSession === null
              ? {}
              : { end_session_endpoint: configuredEndSession }),
          }),
        );
      }
      if (url.endsWith('/userinfo')) {
        if (options.userInfoNow !== undefined) {
          now = options.userInfoNow;
        }
        return Promise.resolve(jsonResponse(options.userInfo ?? { sub: 'subject-123' }));
      }
      return Promise.resolve(jsonResponse(tokenResponses.shift() ?? defaultTokens()));
    });
  const environment: NativeOidcEnvironment = {
    fetch: fetchImplementation,
    storage,
    browser,
    now: () => now,
  };
  return {
    client: new NativeOidcClient(
      {
        issuer: `${issuer}///`,
        clientId: 'product-mobile',
        redirectUri: 'product://oauth',
        offlineAccess: true,
        storageKeyPrefix: 'native-test',
      },
      environment,
    ),
    storage,
    browser,
    requests,
    tokenResponses,
    setNow(value) {
      now = value;
    },
  };
}

function successfulAuthorization(
  overrides: Partial<Extract<AuthorizationResult, { readonly type: 'success' }>> = {},
): Extract<AuthorizationResult, { readonly type: 'success' }> {
  return {
    type: 'success',
    code: 'one-time-code',
    state: 'state-123',
    expectedState: 'state-123',
    codeVerifier: 'pkce-verifier',
    ...overrides,
  };
}

function defaultTokens(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    access_token: 'access-secret',
    refresh_token: 'refresh-secret',
    id_token: 'identity-secret',
    expires_in: 60,
    ...overrides,
  };
}

function storedSession(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    subject: 'stored-subject',
    accessToken: 'stored-access',
    refreshToken: 'stored-refresh',
    idToken: 'stored-id',
    expiresAt: 61_000,
    ...overrides,
  });
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('NativeOidcClient', () => {
  it('discovers standard endpoints, exchanges PKCE, and stores the UserInfo subject', async () => {
    const test = makeHarness();
    const updates: (string | undefined)[] = [];
    test.client.subscribe((session) => updates.push(session?.subject));

    await expect(test.client.signIn()).resolves.toEqual({
      status: 'success',
      subject: 'subject-123',
    });

    expect(test.requests[0]?.url).toBe(`${issuer}/.well-known/openid-configuration`);
    expect(test.browser.authorizationRequests[0]).toEqual({
      authorizationEndpoint: 'https://login.example.test/oauth/authorize',
      clientId: 'product-mobile',
      redirectUri: 'product://oauth',
      scopes: ['openid', 'profile', 'email', 'offline_access'],
    });
    const exchange = test.requests.find(({ url }) => url.endsWith('/token'));
    const exchangeBody = exchange?.init?.body as URLSearchParams;
    expect(exchangeBody.get('code')).toBe('one-time-code');
    expect(exchangeBody.get('code_verifier')).toBe('pkce-verifier');
    const userInfo = test.requests.find(({ url }) => url.endsWith('/userinfo'));
    expect(userInfo?.init?.headers).toEqual({ authorization: 'Bearer access-secret' });
    expect(test.client.session()).toEqual({
      subject: 'subject-123',
      accessToken: 'access-secret',
      refreshToken: 'refresh-secret',
      idToken: 'identity-secret',
      expiresAt: 61_000,
    });
    expect(Object.isFrozen(test.client.session())).toBe(true);
    expect(updates).toEqual([undefined, 'subject-123']);
  });

  it.each(['cancel', 'dismiss'] as const)(
    'returns %s as a non-error cancellation',
    async (type) => {
      const test = makeHarness();
      test.browser.authorizationResults = [{ type }];

      await expect(test.client.signIn()).resolves.toEqual({
        status: 'cancelled',
        reason: type,
      });
      expect(test.client.session()).toBeUndefined();
      expect(test.requests.some(({ url }) => url.endsWith('/token'))).toBe(false);
    },
  );

  it.each([
    [successfulAuthorization({ state: 'attacker-state' }), 'callback_state_mismatch'],
    [successfulAuthorization({ codeVerifier: '' }), 'pkce_verifier_missing'],
  ] as const)('rejects invalid state or PKCE transactions', async (result, code) => {
    const test = makeHarness();
    test.browser.authorizationResults = [result];

    await expect(test.client.signIn()).rejects.toMatchObject({ code });
    expect(test.client.session()).toBeUndefined();
  });

  it('retains the ID token while accepting a rotated refresh token', async () => {
    const test = makeHarness({
      tokenResponses: [
        defaultTokens(),
        defaultTokens({
          access_token: 'refreshed-access',
          refresh_token: 'rotated-refresh',
          id_token: undefined,
          expires_in: 120,
        }),
      ],
    });
    await test.client.signIn();
    test.setNow(40_000);

    await expect(test.client.accessToken()).resolves.toBe('refreshed-access');
    expect(test.client.session()).toMatchObject({
      subject: 'subject-123',
      refreshToken: 'rotated-refresh',
      idToken: 'identity-secret',
    });
    const refresh = test.requests.filter(({ url }) => url.endsWith('/token')).at(-1);
    expect((refresh?.init?.body as URLSearchParams).get('refresh_token')).toBe('refresh-secret');
  });

  it('retains the previous refresh token when refresh omits it', async () => {
    const test = makeHarness({
      tokenResponses: [
        defaultTokens(),
        defaultTokens({
          access_token: 'refreshed-access',
          refresh_token: undefined,
          id_token: undefined,
        }),
      ],
    });
    await test.client.signIn();
    test.setNow(40_000);

    await test.client.accessToken();
    expect(test.client.session()).toMatchObject({
      refreshToken: 'refresh-secret',
      idToken: 'identity-secret',
    });
  });

  it('anchors expiry to token receipt before later UserInfo work', async () => {
    const test = makeHarness({ userInfoNow: 500_000 });

    await test.client.signIn();

    expect(test.client.session()?.expiresAt).toBe(61_000);
  });

  it('shares one refresh across proactive and forced concurrent callers', async () => {
    let resolveRefresh: ((response: Response) => void) | undefined;
    let refreshCalls = 0;
    const test = makeHarness({
      fetchOverride: (url) => {
        if (url.endsWith('/.well-known/openid-configuration')) {
          return Promise.resolve(
            jsonResponse({
              issuer,
              authorization_endpoint: 'https://login.example.test/oauth/authorize',
              token_endpoint: 'https://login.example.test/oauth/token',
              userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
            }),
          );
        }
        refreshCalls += 1;
        return new Promise((resolve) => {
          resolveRefresh = resolve;
        });
      },
    });
    test.storage.values.set(sessionKey, storedSession({ expiresAt: 900 }));

    const proactive = test.client.accessToken();
    const forced = test.client.accessToken({ forceRefresh: true });
    await vi.waitFor(() => {
      expect(refreshCalls).toBe(1);
    });
    resolveRefresh?.(
      jsonResponse(defaultTokens({ access_token: 'shared-access', refresh_token: undefined })),
    );

    await expect(Promise.all([proactive, forced])).resolves.toEqual([
      'shared-access',
      'shared-access',
    ]);
    expect(refreshCalls).toBe(1);
  });

  it('does not report expiry after a session is cleared during a rejected refresh', async () => {
    let resolveRefresh: ((response: Response) => void) | undefined;
    const test = makeHarness({
      fetchOverride: (url) => {
        if (url.endsWith('/.well-known/openid-configuration')) {
          return Promise.resolve(
            jsonResponse({
              issuer,
              authorization_endpoint: 'https://login.example.test/oauth/authorize',
              token_endpoint: 'https://login.example.test/oauth/token',
              userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
            }),
          );
        }
        return new Promise((resolve) => {
          resolveRefresh = resolve;
        });
      },
    });
    test.storage.values.set(sessionKey, storedSession({ expiresAt: 900 }));
    const expired: unknown[] = [];
    test.client.subscribeSessionExpired((event) => expired.push(event));

    const pending = test.client.accessToken();
    await vi.waitFor(() => {
      expect(resolveRefresh).toBeDefined();
    });
    await test.client.clearSession();
    resolveRefresh?.(jsonResponse({ error: 'invalid_grant' }, 400));

    await expect(pending).resolves.toBeUndefined();
    expect(test.client.session()).toBeUndefined();
    expect(expired).toEqual([]);
  });

  it('forces refresh outside the proactive expiry window', async () => {
    const test = makeHarness({
      tokenResponses: [defaultTokens(), defaultTokens({ access_token: 'forced-access' })],
    });
    await test.client.signIn();

    await expect(test.client.accessToken({ forceRefresh: true })).resolves.toBe('forced-access');
    expect(test.requests.filter(({ url }) => url.endsWith('/token'))).toHaveLength(2);
  });

  it('keeps a session retryable after a transient refresh failure', async () => {
    const test = makeHarness({
      fetchOverride: (url) => {
        if (url.endsWith('/.well-known/openid-configuration')) {
          return Promise.resolve(
            jsonResponse({
              issuer,
              authorization_endpoint: 'https://login.example.test/oauth/authorize',
              token_endpoint: 'https://login.example.test/oauth/token',
              userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
            }),
          );
        }
        return Promise.resolve(jsonResponse({ error: 'temporarily_unavailable' }, 503));
      },
    });
    test.storage.values.set(sessionKey, storedSession({ expiresAt: 900 }));

    await expect(test.client.accessToken()).rejects.toMatchObject({
      code: 'refresh_failed',
      retryable: true,
      status: 503,
    });
    expect(test.client.session()?.refreshToken).toBe('stored-refresh');
    expect(test.storage.values.has(sessionKey)).toBe(true);
  });

  it.each([
    [400, { error: 'provider_error' }],
    [403, { error: 'invalid_token' }],
  ] as const)('expires the session for terminal refresh rejection %#', async (status, body) => {
    const test = makeHarness({
      fetchOverride: (url) => {
        if (url.endsWith('/.well-known/openid-configuration')) {
          return Promise.resolve(
            jsonResponse({
              issuer,
              authorization_endpoint: 'https://login.example.test/oauth/authorize',
              token_endpoint: 'https://login.example.test/oauth/token',
              userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
            }),
          );
        }
        return Promise.resolve(jsonResponse(body, status));
      },
    });
    test.storage.values.set(sessionKey, storedSession({ expiresAt: 900 }));
    const expired: unknown[] = [];
    test.client.subscribeSessionExpired((event) => expired.push(event));

    await expect(test.client.accessToken()).resolves.toBeUndefined();
    expect(test.client.session()).toBeUndefined();
    expect(test.storage.values.has(sessionKey)).toBe(false);
    expect(expired).toEqual([{ type: 'session-expired', reason: 'refresh_rejected' }]);
  });

  it('clears an expired session without a refresh token', async () => {
    const test = makeHarness();
    test.storage.values.set(sessionKey, storedSession({ refreshToken: undefined, expiresAt: 900 }));
    const expired: unknown[] = [];
    test.client.subscribeSessionExpired((event) => expired.push(event));

    await expect(test.client.accessToken()).resolves.toBeUndefined();
    expect(test.client.session()).toBeUndefined();
    expect(test.storage.values.has(sessionKey)).toBe(false);
    expect(expired).toEqual([{ type: 'session-expired', reason: 'refresh_unavailable' }]);
  });

  it('deletes corrupt secure storage and initializes signed out', async () => {
    const test = makeHarness();
    test.storage.values.set(sessionKey, '{private-token:broken');

    await expect(test.client.initialize()).resolves.toBeUndefined();
    expect(test.storage.deletes).toContain(sessionKey);
    expect(test.storage.values.has(sessionKey)).toBe(false);
  });

  it('clears the local session before a working end-session flow', async () => {
    const test = makeHarness();
    await test.client.signIn();
    test.browser.beforeEndSession = () => {
      expect(test.client.session()).toBeUndefined();
      expect(test.storage.values.has(sessionKey)).toBe(false);
    };

    await expect(test.client.signOut()).resolves.toEqual({ providerLogout: 'completed' });
    const request = test.browser.endSessionRequests[0];
    const logout = new URL(request?.url ?? 'https://invalid.example.test');
    expect(logout.origin + logout.pathname).toBe('https://login.example.test/session/end');
    expect(logout.searchParams.get('id_token_hint')).toBe('identity-secret');
    expect(test.storage.values.has(forceLoginKey)).toBe(false);
  });

  it('forces the next login after a failed end-session flow and keeps it through cancellation', async () => {
    const test = makeHarness();
    await test.client.signIn();
    test.browser.endSessionError = new Error('provider body access-secret');

    await expect(test.client.signOut()).resolves.toEqual({ providerLogout: 'failed' });
    expect(test.storage.values.get(forceLoginKey)).toBe('1');

    test.browser.endSessionError = undefined;
    test.browser.authorizationResults = [{ type: 'cancel' }];
    await expect(test.client.signIn()).resolves.toEqual({
      status: 'cancelled',
      reason: 'cancel',
    });
    expect(test.browser.authorizationRequests.at(-1)?.prompt).toBe('login');
    expect(test.storage.values.get(forceLoginKey)).toBe('1');
  });

  it('forces the next login when discovery has no end-session endpoint', async () => {
    const test = makeHarness({ endSessionEndpoint: null });
    await test.client.signIn();

    await expect(test.client.signOut()).resolves.toEqual({ providerLogout: 'unavailable' });
    test.browser.authorizationResults = [{ type: 'dismiss' }];
    await test.client.signIn();
    expect(test.browser.authorizationRequests.at(-1)?.prompt).toBe('login');
  });

  it('removes forced login only after a later successful sign-in', async () => {
    const test = makeHarness();
    test.storage.values.set(forceLoginKey, '1');

    await test.client.signIn();

    expect(test.browser.authorizationRequests[0]?.prompt).toBe('login');
    expect(test.storage.values.has(forceLoginKey)).toBe(false);
  });

  it('redacts provider bodies, tokens, codes, and adapter exceptions', async () => {
    const leaked = 'provider-private access-secret one-time-code';
    const requests: string[] = [];
    const fetchImplementation: FetchPort = (url) => {
      requests.push(url);
      if (url.endsWith('/.well-known/openid-configuration')) {
        return Promise.resolve(
          jsonResponse({
            issuer,
            authorization_endpoint: 'https://login.example.test/oauth/authorize',
            token_endpoint: 'https://login.example.test/oauth/token',
            userinfo_endpoint: 'https://login.example.test/oauth/userinfo',
          }),
        );
      }
      return Promise.resolve(new Response(leaked, { status: 400 }));
    };
    const tokenFailure = makeHarness({ fetchOverride: fetchImplementation });
    let caught: unknown;
    try {
      await tokenFailure.client.signIn();
    } catch (cause) {
      caught = cause;
    }
    expect(caught).toBeInstanceOf(OidcError);
    expect(caught).toMatchObject({ code: 'token_exchange_failed', status: 400 });
    expect(String(caught)).not.toContain(leaked);
    expect(safeAuthErrorMessage(caught)).toBe('OIDC token exchange failed.');
    expect(safeAuthErrorMessage(new Error(leaked))).toBe('OIDC login failed.');
    expect(requests).toHaveLength(2);

    const browserFailure = makeHarness();
    browserFailure.browser.authorizationError = new Error(leaked);
    await browserFailure.client.signIn().catch((cause: unknown) => {
      expect(String(cause)).not.toContain(leaked);
      expect(cause).toMatchObject({ code: 'authorization_failed' });
    });
  });
});
