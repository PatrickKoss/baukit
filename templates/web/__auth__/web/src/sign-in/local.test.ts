import type { BrowserSessionEnvironment } from '@baukit/auth-web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createLocalSignIn } from './local';

const ORIGIN = 'https://app.example.test';

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => {
      values.clear();
    },
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `header.${encoded.replace(/=+$/u, '')}.signature`;
}

const realmClaims = {
{% if context.auth_clerk %}  sub: 'local-user',
  sid: 'local-session',
  azp: 'web',
  v: 2,
  o: { id: 'org_local', rol: 'admin', slg: 'local' },
{% else %}  sub: 'local-user',
  sid: 'local-session',
  client_id: 'client_local',
  org_id: 'org_local',
  role: 'admin',
  permissions: ['items:read', 'items:write'],
{% endif %}};

const expectedSession = {
  sessionId: 'local-session',
  user: { id: 'local-user' },
{% if context.auth_clerk %}  organization: { id: 'org_local', role: 'admin', slug: 'local', permissions: [] },
{% else %}  organization: { id: 'org_local', role: 'admin', permissions: ['items:read', 'items:write'] },
{% endif %}};

interface Harness {
  readonly environment: BrowserSessionEnvironment;
  readonly localStorage: Storage;
  readonly fetch: ReturnType<typeof vi.fn<(url: string, init?: RequestInit) => Promise<Response>>>;
  readonly navigations: string[];
  currentUrl: string;
}

function harness(): Harness {
  const navigations: string[] = [];
  const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>((url) => {
    if (url.endsWith('/.well-known/openid-configuration')) {
      const issuer = url.replace('/.well-known/openid-configuration', '');
      return Promise.resolve(
        Response.json({
          issuer,
          authorization_endpoint: `${issuer}/protocol/openid-connect/auth`,
          token_endpoint: `${issuer}/protocol/openid-connect/token`,
          end_session_endpoint: `${issuer}/protocol/openid-connect/logout`,
        }),
      );
    }
    if (url.endsWith('/auth/session/exchange')) {
      return Promise.resolve(
        Response.json({
          accessToken: unsignedJwt(realmClaims),
          expiresAt: Date.now() + 300_000,
          subject: 'local-user',
        }),
      );
    }
    if (url.endsWith('/auth/session/logout')) {
      return Promise.resolve(
        Response.json({ endSessionUrl: 'http://localhost:8080/realms/product/protocol/openid-connect/logout' }),
      );
    }
    return Promise.resolve(new Response(null, { status: 404 }));
  });
  const localStorage = memoryStorage();
  const state: Harness = {
    navigations,
    fetch,
    localStorage,
    currentUrl: `${ORIGIN}/`,
    environment: {
      fetch: (input, init) =>
        fetch(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, init),
      crypto: globalThis.crypto,
      localStorage,
      sessionStorage: memoryStorage(),
      currentUrl: () => state.currentUrl,
      navigate: (url) => {
        navigations.push(url);
      },
      replaceUrl: (url) => {
        state.currentUrl = url;
      },
      now: () => Date.now(),
    },
  };
  return state;
}

beforeEach(() => {
  vi.stubGlobal('window', { location: { origin: ORIGIN } });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('local Keycloak sign-in adapter', () => {
  it('signs in with authorization code and S256 PKCE against the local realm', async () => {
    const run = harness();
    const signIn = createLocalSignIn(run.environment);

    expect(signIn.providerLabel).toBe('local Keycloak');
    expect(signIn.hasSession()).toBe(false);
    await expect(signIn.session()).resolves.toBeUndefined();
    await signIn.signIn();

    const authorization = new URL(run.navigations[0] ?? '');
    expect(authorization.pathname).toMatch(/\/realms\/[^/]+\/protocol\/openid-connect\/auth$/u);
    expect(authorization.hostname).toBe('localhost');
    expect(authorization.searchParams.get('client_id')).toMatch(/-web$/u);
    expect(authorization.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authorization.searchParams.get('scope')).toBe('openid');

    run.currentUrl = `${ORIGIN}/?code=local-code&state=${authorization.searchParams.get('state') ?? ''}`;
    await expect(signIn.handleCallback()).resolves.toBe(true);

    const exchange = run.fetch.mock.calls.find(([url]) => url.endsWith('/auth/session/exchange'));
    expect(exchange?.[0]).toMatch(/^http:\/\/localhost:\d+\/auth\/session\/exchange$/u);
    expect(exchange?.[1]?.credentials).toBe('include');
    expect(exchange?.[1]?.headers).toMatchObject({ 'X-Baukit-Session': '1' });
    const rawBody = exchange?.[1]?.body;
    expect(typeof rawBody).toBe('string');
    const body: unknown = JSON.parse(typeof rawBody === 'string' ? rawBody : '');
    expect(body).toMatchObject({ code: 'local-code', codeVerifier: expect.stringMatching(/^[\w-]{43,}$/u) as unknown });
    expect(run.fetch.mock.calls.some(([url]) => url.endsWith('/token'))).toBe(false);
    const stored = Array.from({ length: run.localStorage.length }, (_, index) =>
      run.localStorage.getItem(run.localStorage.key(index) ?? ''),
    ).join('\n');
    expect(stored).not.toContain('header.');
    expect(signIn.hasSession()).toBe(true);
    await expect(signIn.session()).resolves.toEqual(expectedSession);
  });

  it('signs out through the backend, which returns the realm end-session URL', async () => {
    const run = harness();
    const signIn = createLocalSignIn(run.environment);

    await expect(signIn.signOut()).resolves.toBe(true);

    expect(run.fetch.mock.calls.some(([url]) => url.endsWith('/auth/session/logout'))).toBe(true);
    expect(new URL(run.navigations[0] ?? '').pathname).toMatch(/\/protocol\/openid-connect\/logout$/u);
    expect(signIn.hasSession()).toBe(false);
  });

  it('expires the session when profile erasure clears it', () => {
    const run = harness();
    const signIn = createLocalSignIn(run.environment);
    const listener = vi.fn();
    const unsubscribe = signIn.subscribeSessionExpired(listener);

    signIn.clearSession({ reason: 'profile_erased' });
    unsubscribe();

    expect(listener).toHaveBeenCalledWith({ type: 'session-expired', reason: 'profile_erased' });
  });
});
