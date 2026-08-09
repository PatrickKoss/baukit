import { describe, expect, it } from 'vitest';

import {
  OidcClient,
  OidcError,
  buildAuthorizationUrl,
  normalizeIssuer,
  safeAuthErrorMessage,
  type OidcClientEnvironment,
} from './index.js';

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  public get length(): number {
    return this.values.size;
  }

  public clear(): void {
    this.values.clear();
  }

  public getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  public key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }

  public removeItem(key: string): void {
    this.values.delete(key);
  }

  public setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

interface TestEnvironment {
  readonly environment: OidcClientEnvironment;
  readonly localStorage: MemoryStorage;
  readonly sessionStorage: MemoryStorage;
  readonly navigations: string[];
  readonly replacements: string[];
  setCurrentUrl(url: string): void;
  setNow(value: number): void;
}

function makeEnvironment(fetchImplementation: typeof globalThis.fetch): TestEnvironment {
  const localStorage = new MemoryStorage();
  const sessionStorage = new MemoryStorage();
  const navigations: string[] = [];
  const replacements: string[] = [];
  let currentUrl = 'https://app.example.test/';
  let now = 1_000_000;
  return {
    environment: {
      fetch: fetchImplementation,
      crypto: globalThis.crypto,
      localStorage,
      sessionStorage,
      currentUrl: () => currentUrl,
      navigate: (url) => navigations.push(url),
      replaceUrl: (url) => replacements.push(url),
      now: () => now,
    },
    localStorage,
    sessionStorage,
    navigations,
    replacements,
    setCurrentUrl: (url) => {
      currentUrl = url;
    },
    setNow: (value) => {
      now = value;
    },
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function required<T>(value: T | null | undefined): T {
  if (value === undefined || value === null) {
    throw new Error('Expected test value to be present.');
  }
  return value;
}

describe('OidcClient', () => {
  it('discovers endpoints and completes login, deduplicated callback, refresh, and logout', async () => {
    const requests: { readonly url: string; readonly init: RequestInit | undefined }[] = [];
    let tokenCalls = 0;
    const mockedFetch: typeof globalThis.fetch = (input, init) => {
      const url = input instanceof Request ? input.url : input.toString();
      requests.push({ url, init });
      if (url.endsWith('/.well-known/openid-configuration')) {
        return Promise.resolve(
          jsonResponse({
            issuer: 'https://identity.example.test/tenant',
            authorization_endpoint: 'https://login.example.test/oauth/authorize',
            token_endpoint: 'https://login.example.test/oauth/token',
            end_session_endpoint: 'https://login.example.test/session/end',
          }),
        );
      }
      tokenCalls += 1;
      return Promise.resolve(
        tokenCalls === 1
          ? jsonResponse({
              access_token: 'access-secret',
              refresh_token: 'refresh-secret',
              id_token: 'identity-secret',
              expires_in: 60,
            })
          : jsonResponse({ access_token: 'refreshed-secret', expires_in: 120 }),
      );
    };
    const test = makeEnvironment(mockedFetch);
    const client = new OidcClient(
      {
        issuer: 'https://identity.example.test/tenant///',
        clientId: 'product-web',
        redirectUri: 'https://app.example.test/callback',
        scopes: ['profile', 'email', 'profile'],
        offlineAccess: true,
        storageKeyPrefix: 'test-auth',
      },
      test.environment,
    );

    await client.login();
    const authorization = new URL(required(test.navigations[0]));
    expect(authorization.origin + authorization.pathname).toBe(
      'https://login.example.test/oauth/authorize',
    );
    expect(authorization.searchParams.get('scope')).toBe('openid profile email offline_access');
    expect(authorization.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authorization.searchParams.get('code_challenge')).toMatch(/^[\w-]{43}$/u);

    const transaction = JSON.parse(
      required(test.sessionStorage.getItem('test-auth:transaction')),
    ) as {
      readonly state: string;
    };
    test.setCurrentUrl(
      `https://app.example.test/callback?code=one-time-code&state=${transaction.state}&session_state=private&iss=https%3A%2F%2Fidentity.example.test%2Ftenant&kept=yes`,
    );
    const firstCallback = client.handleCallback();
    const repeatedCallback = client.handleCallback();
    expect(firstCallback).toBe(repeatedCallback);
    await expect(Promise.all([firstCallback, repeatedCallback])).resolves.toEqual([true, true]);
    expect(tokenCalls).toBe(1);
    expect(test.replacements).toEqual(['https://app.example.test/callback?kept=yes']);
    expect(await client.accessToken()).toBe('access-secret');

    test.setNow(1_031_000);
    await expect(Promise.all([client.accessToken(), client.accessToken()])).resolves.toEqual([
      'refreshed-secret',
      'refreshed-secret',
    ]);
    expect(tokenCalls).toBe(2);
    const refreshBody = requests.at(-1)?.init?.body as URLSearchParams;
    expect(refreshBody.get('grant_type')).toBe('refresh_token');
    expect(refreshBody.get('refresh_token')).toBe('refresh-secret');

    await expect(client.logout()).resolves.toBe(true);
    const logout = new URL(required(test.navigations.at(-1)));
    expect(logout.origin + logout.pathname).toBe('https://login.example.test/session/end');
    expect(logout.searchParams.get('id_token_hint')).toBe('identity-secret');
    expect(client.hasSession()).toBe(false);
    expect(requests.filter(({ url }) => url.includes('.well-known')).length).toBe(1);
  });

  it('uses only discovered endpoints and makes offline access opt-in', () => {
    const url = buildAuthorizationUrl(
      {
        clientId: 'client',
        redirectUri: 'https://app.example.test/callback',
        scopes: ['custom'],
      },
      'https://provider.example.test/non-keycloak/authorize',
      { state: 'state', challenge: 'challenge' },
    );
    expect(url.pathname).toBe('/non-keycloak/authorize');
    expect(url.searchParams.get('scope')).toBe('openid custom');
    expect(url.searchParams.has('offline_access')).toBe(false);
    expect(normalizeIssuer('https://provider.example.test/path///')).toBe(
      'https://provider.example.test/path',
    );
  });

  it('sanitizes provider errors, response content, and unknown errors', async () => {
    const leakedDescription = 'private content access-token-secret';
    const fetchImplementation: typeof globalThis.fetch = () =>
      Promise.resolve(
        jsonResponse({
          issuer: 'https://identity.example.test',
          authorization_endpoint: 'https://identity.example.test/authorize',
          token_endpoint: 'https://identity.example.test/token',
        }),
      );
    const test = makeEnvironment(fetchImplementation);
    test.sessionStorage.setItem(
      'safe:transaction',
      JSON.stringify({ state: 'expected', verifier: 'verifier' }),
    );
    test.setCurrentUrl(
      `https://app.example.test/callback?error=access_denied&error_description=${encodeURIComponent(leakedDescription)}&state=expected`,
    );
    const client = new OidcClient(
      {
        issuer: 'https://identity.example.test',
        clientId: 'client',
        redirectUri: 'https://app.example.test/callback',
        storageKeyPrefix: 'safe',
      },
      test.environment,
    );

    let caught: unknown;
    try {
      await client.handleCallback();
    } catch (cause) {
      caught = cause;
    }
    expect(caught).toBeInstanceOf(OidcError);
    expect(safeAuthErrorMessage(caught)).toBe('OIDC authorization failed.');
    expect(safeAuthErrorMessage(caught)).not.toContain(leakedDescription);
    expect(safeAuthErrorMessage(new Error(leakedDescription))).toBe('OIDC login failed.');
    expect(test.replacements[0]).not.toContain('error_description');
  });

  it('does not expose a token endpoint error body or authorization code', async () => {
    const fetchImplementation: typeof globalThis.fetch = (input) => {
      const url = input instanceof Request ? input.url : input.toString();
      return Promise.resolve(
        url.includes('.well-known')
          ? jsonResponse({
              issuer: 'https://identity.example.test',
              authorization_endpoint: 'https://identity.example.test/authorize',
              token_endpoint: 'https://identity.example.test/token',
            })
          : new Response('private-content access-token-secret one-time-code', { status: 400 }),
      );
    };
    const test = makeEnvironment(fetchImplementation);
    test.sessionStorage.setItem(
      'http-error:transaction',
      JSON.stringify({ state: 'expected', verifier: 'verifier' }),
    );
    test.setCurrentUrl('https://app.example.test/callback?code=one-time-code&state=expected');
    const client = new OidcClient(
      {
        issuer: 'https://identity.example.test',
        clientId: 'client',
        redirectUri: 'https://app.example.test/callback',
        storageKeyPrefix: 'http-error',
      },
      test.environment,
    );

    let caught: unknown;
    try {
      await client.handleCallback();
    } catch (cause) {
      caught = cause;
    }
    expect(caught).toMatchObject({ code: 'token_exchange_failed', status: 400 });
    expect(safeAuthErrorMessage(caught)).toBe('OIDC token exchange failed.');
    expect(String(caught)).not.toContain('private-content');
    expect(String(caught)).not.toContain('one-time-code');
  });

  it('rejects mismatched discovery issuers without exposing metadata content', async () => {
    const fetchImplementation: typeof globalThis.fetch = () =>
      Promise.resolve(
        jsonResponse({
          issuer: 'https://attacker.example.test/token-secret',
          authorization_endpoint: 'https://attacker.example.test/authorize',
          token_endpoint: 'https://attacker.example.test/token',
        }),
      );
    const test = makeEnvironment(fetchImplementation);
    const client = new OidcClient(
      {
        issuer: 'https://identity.example.test',
        clientId: 'client',
        redirectUri: 'https://app.example.test/callback',
      },
      test.environment,
    );
    await expect(client.discover()).rejects.toMatchObject({ code: 'issuer_mismatch' });
    await client.discover().catch((error: unknown) => {
      expect(safeAuthErrorMessage(error)).toBe(
        'OIDC provider issuer did not match the configured issuer.',
      );
      expect(safeAuthErrorMessage(error)).not.toContain('token-secret');
    });
  });
});
