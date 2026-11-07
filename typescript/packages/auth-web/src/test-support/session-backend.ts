import { expect } from 'vitest';

import { BrowserSessionClient, type BrowserSessionClientConfig } from '../index.js';
import {
  discoveryResponse,
  fakeBrowser,
  jsonResponse,
  requestUrl,
  required,
  type Browser,
  type Tab,
} from './browser-session.js';

export const API = 'https://api.example.test';
export const ACCESS_TOKEN = 'eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEifQ.signature-one';
export const ROTATED_TOKEN = 'eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEifQ.signature-two';
export const LIFETIME_MS = 300_000;

export const CONFIG: BrowserSessionClientConfig = {
  issuer: 'https://identity.example.test/realm',
  clientId: 'product-web',
  redirectUri: 'https://app.example.test/',
  storageKeyPrefix: 'product',
  apiBaseUrl: `${API}/`,
};

export interface Backend {
  readonly fetch: typeof globalThis.fetch;
  readonly calls: { readonly url: string; readonly init: RequestInit | undefined }[];
  refresh: () => Response | Promise<Response>;
  logout: () => Response;
}

export function backend(browserNow: () => number, lifetimeMs = LIFETIME_MS): Backend {
  const state: Backend = {
    calls: [],
    refresh: () =>
      jsonResponse({
        accessToken: ROTATED_TOKEN,
        expiresAt: browserNow() + lifetimeMs,
        subject: 'user-1',
      }),
    logout: () => new Response(null, { status: 204 }),
    fetch: (input, init) => {
      const url = requestUrl(input);
      state.calls.push({ url, init });
      if (url.endsWith('/.well-known/openid-configuration')) {
        return Promise.resolve(discoveryResponse());
      }
      if (url === `${API}/auth/session/exchange`) {
        return Promise.resolve(
          jsonResponse({
            accessToken: ACCESS_TOKEN,
            expiresAt: browserNow() + lifetimeMs,
            subject: 'user-1',
          }),
        );
      }
      if (url === `${API}/auth/session/refresh`) {
        return Promise.resolve(state.refresh());
      }
      if (url === `${API}/auth/session/logout`) {
        return Promise.resolve(state.logout());
      }
      return Promise.resolve(new Response('{}', { status: 200 }));
    },
  };
  return state;
}

export function setup(lifetimeMs = LIFETIME_MS): {
  browser: Browser;
  server: Backend;
  now: () => number;
} {
  const clock = { browser: undefined as Browser | undefined };
  const now = () => required(clock.browser).now();
  const server = backend(now, lifetimeMs);
  const browser = fakeBrowser((input, init) => server.fetch(input, init));
  clock.browser = browser;
  return { browser, server, now };
}

export async function signIn(
  tab: Tab,
  client = new BrowserSessionClient(CONFIG, tab.environment),
): Promise<BrowserSessionClient> {
  await client.login();
  const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
  tab.setCurrentUrl(`https://app.example.test/?code=one-time-code&state=${state}`);
  await expect(client.handleCallback()).resolves.toBe(true);
  return client;
}

export function storageEntries(storage: Storage): [string, string | null][] {
  return Array.from({ length: storage.length }, (_, index) => {
    const key = storage.key(index) ?? '';
    return [key, storage.getItem(key)];
  });
}

export function sessionCalls(server: Backend, endpoint: string): Backend['calls'] {
  return server.calls.filter(({ url }) => url === `${API}/auth/session/${endpoint}`);
}
