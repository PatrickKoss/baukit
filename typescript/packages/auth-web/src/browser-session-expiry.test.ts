import { describe, expect, it } from 'vitest';

import { jsonResponse, requestUrl } from './test-support/browser-session.js';
import {
  ACCESS_TOKEN,
  ROTATED_TOKEN,
  sessionCalls,
  setup,
  signIn,
} from './test-support/session-backend.js';

const HOUR_MS = 3_600_000;
const LIFETIME_S = 300;

describe('BrowserSessionClient expiry', () => {
  it('trusts expiresIn over expiresAt when the server clock is skewed', async () => {
    const { browser, server, now } = setup();
    const skewed = () => ({
      expiresAt: now() - HOUR_MS + LIFETIME_S * 1000,
      expiresIn: LIFETIME_S,
      subject: 'user-1',
    });
    const original = server.fetch;
    server.fetch = (input, init) => {
      if (requestUrl(input).endsWith('/auth/session/exchange')) {
        return Promise.resolve(jsonResponse({ accessToken: ACCESS_TOKEN, ...skewed() }));
      }
      return original(input, init);
    };
    server.refresh = () => jsonResponse({ accessToken: ROTATED_TOKEN, ...skewed() });
    const client = await signIn(browser.tab());

    await expect(client.getAccessToken()).resolves.toBe(ACCESS_TOKEN);
    expect(sessionCalls(server, 'refresh')).toHaveLength(0);
  });

  it('rejects a session with a negative expiresIn', async () => {
    const { browser, server } = setup();
    server.refresh = () =>
      jsonResponse({ accessToken: ROTATED_TOKEN, expiresAt: 1, expiresIn: -1, subject: 'u' });
    const client = await signIn(browser.tab());
    await expect(client.getAccessToken({ forceRefresh: true })).rejects.toThrow();
  });
});
