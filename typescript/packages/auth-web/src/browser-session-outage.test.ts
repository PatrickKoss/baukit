import { afterEach, describe, expect, it, vi } from 'vitest';

import { BrowserSessionClient, REFRESH_TIMEOUT_MS } from './browser-session-client.js';
import { OidcError } from './oidc-errors.js';
import { problem, requestUrl } from './test-support/browser-session.js';
import {
  ACCESS_TOKEN,
  API,
  CONFIG,
  LIFETIME_MS,
  ROTATED_TOKEN,
  sessionCalls,
  setup,
  signIn,
} from './test-support/session-backend.js';

const SIGNED_IN_AT = 1_000_000;
const INSIDE_REFRESH_WINDOW = SIGNED_IN_AT + LIFETIME_MS - 10_000;
const AFTER_EXPIRY = SIGNED_IN_AT + LIFETIME_MS + 1;

afterEach(() => {
  vi.useRealTimers();
});

describe('BrowserSessionClient refresh under outage', () => {
  it.each([
    { name: 'a network error', response: () => Promise.reject(new TypeError('offline')) },
    { name: 'a 503', response: () => Promise.resolve(problem(503, 'idp_unavailable')) },
    { name: 'a 429', response: () => Promise.resolve(problem(429, 'rate_limited')) },
  ])(
    'keeps the unexpired token after $name, then rejects once it expires',
    async ({ response }) => {
      const { browser, server } = setup();
      const client = await signIn(browser.tab());
      server.refresh = response;

      browser.setNow(INSIDE_REFRESH_WINDOW);
      await expect(client.getAccessToken()).resolves.toBe(ACCESS_TOKEN);
      expect(sessionCalls(server, 'refresh')).toHaveLength(1);

      browser.setNow(AFTER_EXPIRY);
      await expect(client.getAccessToken()).rejects.toMatchObject({
        code: 'refresh_failed',
        retryable: true,
      });
      expect(client.hasSession()).toBe(true);
    },
  );

  it('still rejects a non-retryable failure while the token is valid', async () => {
    const { browser, server } = setup();
    const client = await signIn(browser.tab());
    server.refresh = () => problem(403, 'csrf_rejected');
    browser.setNow(INSIDE_REFRESH_WINDOW);

    await expect(client.getAccessToken()).rejects.toMatchObject({ retryable: false });
  });

  it('still ends the session on 401', async () => {
    const { browser, server } = setup();
    const client = await signIn(browser.tab());
    server.refresh = () => problem(401, 'refresh_rejected');
    browser.setNow(INSIDE_REFRESH_WINDOW);

    await expect(client.getAccessToken()).resolves.toBeUndefined();
    expect(client.hasSession()).toBe(false);
  });

  it('aborts a hanging refresh after 10 s and releases the refresh lock', async () => {
    vi.useFakeTimers();
    const { browser, server } = setup();
    await signIn(browser.tab());
    const tab = browser.tab();
    let hang = true;
    const fetch: typeof globalThis.fetch = (input, init) => {
      if (!hang || requestUrl(input) !== `${API}/auth/session/refresh`) {
        return server.fetch(input, init);
      }
      return new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    };
    const client = new BrowserSessionClient(CONFIG, { ...tab.environment, fetch });

    const pending = client.getAccessToken().catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(REFRESH_TIMEOUT_MS);
    const failure = await pending;

    expect(failure).toBeInstanceOf(OidcError);
    expect(failure).toMatchObject({ code: 'refresh_failed', retryable: true });
    expect(client.hasSession()).toBe(true);
    hang = false;
    await expect(client.getAccessToken()).resolves.toBe(ROTATED_TOKEN);
  });
});
