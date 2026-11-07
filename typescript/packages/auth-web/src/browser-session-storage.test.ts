import { afterEach, describe, expect, it, vi } from 'vitest';

import { BrowserSessionClient } from './index.js';
import {
  MemoryStorage,
  discoveryResponse,
  fakeBrowser,
  jsonResponse,
  requestUrl,
  required,
} from './test-support/browser-session.js';

const ACCESS_TOKEN = 'eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEifQ.access-signature';
const FORBIDDEN = [ACCESS_TOKEN, 'refresh_token', 'eyJ'];

/** A stand-in for IndexedDB or the Cache API that records every call's arguments. */
function recordingApi(writes: string[], path: string): unknown {
  const call = (...args: unknown[]): unknown => {
    writes.push(`${path}(${args.map((arg) => safeText(arg)).join(',')})`);
    return recordingApi(writes, `${path}()`);
  };
  return new Proxy(call, {
    get: (_target, property) =>
      property === 'then' ? undefined : recordingApi(writes, `${path}.${String(property)}`),
  });
}

function safeText(value: unknown): string {
  try {
    return typeof value === 'string' ? value : JSON.stringify(value);
  } catch {
    return String(value);
  }
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('BrowserSessionClient storage', () => {
  it('never writes a token to localStorage, sessionStorage, IndexedDB or the Cache API', async () => {
    const writes: string[] = [];
    vi.stubGlobal('indexedDB', recordingApi(writes, 'indexedDB'));
    vi.stubGlobal('caches', recordingApi(writes, 'caches'));
    const setItem = vi.spyOn(MemoryStorage.prototype, 'setItem');
    let refreshStatus = 200;
    const browser = fakeBrowser((input) => {
      const url = requestUrl(input);
      if (url.includes('.well-known')) return Promise.resolve(discoveryResponse());
      if (url.endsWith('/logout')) return Promise.resolve(new Response(null, { status: 204 }));
      if (url.endsWith('/refresh') && refreshStatus === 401) {
        return Promise.resolve(jsonResponse({ code: 'session_expired' }, 401));
      }
      return Promise.resolve(
        jsonResponse({
          accessToken: ACCESS_TOKEN,
          expiresAt: 9_000_000,
          subject: 'user-1',
          refresh_token: 'leaked-by-a-broken-backend',
        }),
      );
    });

    const tab = browser.tab();
    const client = new BrowserSessionClient(clientConfig(), tab.environment);
    const others = [browser.tab(), browser.tab()].map(
      (other) => new BrowserSessionClient(clientConfig(), other.environment),
    );
    await client.login();
    const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
    tab.setCurrentUrl(`https://app.example.test/?code=code&state=${state}`);
    await client.handleCallback();
    await Promise.all(
      [client, ...others].map((each) => each.getAccessToken({ forceRefresh: true })),
    );
    refreshStatus = 401;
    await client.getAccessToken({ forceRefresh: true });
    await client.login();
    await client.logout();

    expect(setItem).toHaveBeenCalled();
    const stored = [
      ...setItem.mock.calls.map(([key, value]) => `${key}=${value}`),
      ...writes,
      ...dump(browser.localStorage),
      ...dump(tab.sessionStorage),
    ];
    for (const entry of stored) {
      for (const forbidden of FORBIDDEN) {
        expect(entry, entry).not.toContain(forbidden);
      }
    }
    expect(browser.hub.posted.length).toBeGreaterThan(0);
  });
});

function clientConfig() {
  return {
    issuer: 'https://identity.example.test/realm',
    clientId: 'product-web',
    redirectUri: 'https://app.example.test/',
    storageKeyPrefix: 'product',
    apiBaseUrl: 'https://api.example.test',
  };
}

function dump(storage: Storage): string[] {
  return Array.from({ length: storage.length }, (_, index) => {
    const key = required(storage.key(index));
    return `${key}=${storage.getItem(key) ?? ''}`;
  });
}
