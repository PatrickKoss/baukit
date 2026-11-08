import { afterEach, beforeEach, describe, expect, it } from '@jest/globals';
import * as SecureStore from 'expo-secure-store';
import { Platform } from 'react-native';

import type { SignInClient } from './sign-in/port';

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
  deleteItemAsync: jest.fn(),
}));
jest.mock('expo-crypto', () => ({ getRandomBytesAsync: (size: number) => Promise.resolve(new Uint8Array(size)) }));
jest.mock('expo-web-browser', () => ({ maybeCompleteAuthSession: jest.fn() }));
jest.mock('expo-auth-session', () => ({
  makeRedirectUri: ({ scheme, path }: { scheme: string; path: string }) => `${scheme}://${path}`,
}));

{% if context.pwa and not context.web and not context.auth_oidc %}const mockVendorConfigs: unknown[] = [];
jest.mock('{{ "@baukit/auth-web/clerk" if context.auth_clerk else "@baukit/auth-web/workos" }}', () => ({
  {{ "ClerkWebClient" if context.auth_clerk else "WorkOsWebClient" }}: class {
    public constructor(config: unknown) {
      mockVendorConfigs.push(config);
    }
    public login = () => Promise.resolve();
    public handleCallback = () => Promise.resolve(false);
    public accessToken = () => Promise.resolve(undefined);
    public subscribeSessionExpired = () => () => undefined;
    public logout = () => Promise.resolve(true);
    public clearSession = () => undefined;
  },
}));
jest.mock('expo-constants', () => ({
  __esModule: true,
  default: { expoConfig: { extra: { clerkPublishableKey: 'pk_test_web', workosClientId: 'client_web' } } },
}));

{% endif %}const ORIGIN = 'http://localhost:4100';
/** Each test loads a fresh module registry, including Expo's lazily installed URL globals. */
const FRESH_REGISTRY_TIMEOUT_MS = 30_000;

jest.setTimeout(FRESH_REGISTRY_TIMEOUT_MS);

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `eyJhbGciOiJub25lIn0.${encoded.replace(/=+$/u, '')}.signature`;
}

const accessToken = unsignedJwt({ sub: 'web-user', sid: 'web-session', exp: Math.floor(Date.now() / 1000) + 300 });

interface RecordingStorage extends Storage {
  readonly writes: string[];
}

function recordingStorage(): RecordingStorage {
  const values = new Map<string, string>();
  const writes: string[] = [];
  return {
    writes,
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
      writes.push(`${key}=${value}`);
      values.set(key, value);
    },
  };
}

interface BrowserHarness {
  readonly localStorage: RecordingStorage;
  readonly sessionStorage: RecordingStorage;
  readonly navigations: string[];
  readonly requests: string[];
  readonly databases: string[];
  readonly caches: string[];
  href: string;
}

const overridden = ['window', 'fetch', 'indexedDB', 'caches', 'BroadcastChannel'] as const;
const originals = new Map<string, PropertyDescriptor | undefined>();

function define(name: string, value: unknown): void {
  if (!originals.has(name)) originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}

function installBrowser(): BrowserHarness {
  const harness: BrowserHarness = {
    localStorage: recordingStorage(),
    sessionStorage: recordingStorage(),
    navigations: [],
    requests: [],
    databases: [],
    caches: [],
    href: `${ORIGIN}/`,
  };
  define('window', {
    localStorage: harness.localStorage,
    sessionStorage: harness.sessionStorage,
    location: {
      origin: ORIGIN,
      get href() {
        return harness.href;
      },
      assign: (url: string) => {
        harness.navigations.push(url);
      },
    },
    history: {
      replaceState: (_state: unknown, _title: string, url: string) => {
        harness.href = url;
      },
    },
  });
  define('indexedDB', {
    open: (name: string) => {
      harness.databases.push(name);
      throw new Error('IndexedDB is not part of sign-in');
    },
  });
  define('caches', {
    open: (name: string) => {
      harness.caches.push(name);
      return Promise.reject(new Error('Cache storage is not part of sign-in'));
    },
  });
  define('BroadcastChannel', undefined);
  define('fetch', (input: string, init?: RequestInit) => {
    harness.requests.push(input);
    if (input.endsWith('/.well-known/openid-configuration')) {
      const issuer = input.replace('/.well-known/openid-configuration', '');
      return Promise.resolve(
        Response.json({
          issuer,
          authorization_endpoint: `${issuer}/protocol/openid-connect/auth`,
          token_endpoint: `${issuer}/protocol/openid-connect/token`,
        }),
      );
    }
    if (input.endsWith('/auth/session/exchange') && init?.credentials === 'include') {
      return Promise.resolve(
        Response.json({ accessToken, expiresAt: Date.now() + 300_000, subject: 'web-user' }),
      );
    }
    return Promise.resolve(new Response(null, { status: 404 }));
  });
  return harness;
}

/** Loads a module in a fresh registry whose `react-native` reports the web platform. */
function webModule<T>(load: () => T): T {
  let loaded: T | undefined;
  jest.isolateModules(() => {
    jest.replaceProperty(jest.requireActual<typeof import('react-native')>('react-native').Platform, 'OS', 'web');
    loaded = load();
  });
  if (loaded === undefined) throw new Error('The module did not load.');
  return loaded;
}

function webClient(): SignInClient {
{% if context.auth_oidc %}  return webModule(() => jest.requireActual<typeof import('./auth')>('./auth').authClient);
{% else %}  return webModule(
    () => jest.requireActual<typeof import('./sign-in/local')>('./sign-in/local').signInAdapter.client,
  );
{% endif %}}

beforeEach(() => {
  jest.replaceProperty(Platform, 'OS', 'web');
});

afterEach(() => {
  jest.restoreAllMocks();
  for (const name of overridden) {
    const original = originals.get(name);
    if (original === undefined) Reflect.deleteProperty(globalThis, name);
    else Object.defineProperty(globalThis, name, original);
  }
  originals.clear();
});

describe('web build sign-in', () => {
{% if context.pwa and not context.web %}  it('keeps the access token in memory and writes nothing token-shaped to browser storage', async () => {
    const browser = installBrowser();
    const client = webClient();

    await expect(client.signIn()).resolves.toEqual({ status: 'cancelled', reason: 'dismiss' });
    const authorization = new URL(browser.navigations[0] ?? '');
    expect(authorization.searchParams.get('client_id')).toMatch(/-web$/u);
    expect(authorization.searchParams.get('redirect_uri')).toBe(`${ORIGIN}/`);
    browser.href = `${ORIGIN}/?code=web-code&state=${authorization.searchParams.get('state') ?? ''}`;

    const session = await client.initialize();

    expect(session).toMatchObject({ subject: 'web-user', accessToken });
    await expect(client.accessToken()).resolves.toBe(accessToken);
    expect(browser.requests.some((url) => url.endsWith('/auth/session/exchange'))).toBe(true);
    expect(browser.requests.some((url) => url.endsWith('/token'))).toBe(false);
    const written = [...browser.localStorage.writes, ...browser.sessionStorage.writes].join('\n');
    expect(written).not.toContain(accessToken);
    expect(written).not.toContain('eyJ');
    expect(written).not.toMatch(/refresh/iu);
    expect(browser.databases).toEqual([]);
    expect(browser.caches).toEqual([]);
    expect(SecureStore.setItemAsync).not.toHaveBeenCalled();
    expect(SecureStore.getItemAsync).not.toHaveBeenCalled();
  });
{% if not context.auth_oidc %}
  it('hands production web builds to the {{ "Clerk" if context.auth_clerk else "WorkOS" }} browser SDK instead of a token cache', async () => {
    const browser = installBrowser();
    const adapter = webModule(
      () => jest.requireActual<typeof import('./sign-in/production')>('./sign-in/production').signInAdapter,
    );

    expect(adapter.Provider).toBeUndefined();
    await expect(adapter.client.initialize()).resolves.toBeUndefined();
    expect(mockVendorConfigs).toEqual([
      expect.objectContaining({ {{ "publishableKey: 'pk_test_web'" if context.auth_clerk else "clientId: 'client_web'" }}, redirectUri: `${ORIGIN}/` }),
    ]);
    expect(browser.localStorage.writes).toEqual([]);
    expect(SecureStore.getItemAsync).not.toHaveBeenCalled();
    expect(SecureStore.setItemAsync).not.toHaveBeenCalled();
  });
{% endif %}{% else %}  it('refuses token storage instead of falling back to browser storage', async () => {
    const browser = installBrowser();
    const client = webClient();

    await expect(client.initialize()).rejects.toMatchObject({ code: 'storage_failed' });
    expect(browser.localStorage.writes).toEqual([]);
    expect(browser.sessionStorage.writes).toEqual([]);
    expect(browser.databases).toEqual([]);
    expect(browser.caches).toEqual([]);
    expect(SecureStore.getItemAsync).not.toHaveBeenCalled();
  });
{% endif %}});
