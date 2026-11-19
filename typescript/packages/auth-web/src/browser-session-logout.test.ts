import { describe, expect, it } from 'vitest';

import { BrowserSessionClient } from './index.js';
import { jsonResponse, required } from './test-support/browser-session.js';
import { CONFIG, sessionCalls, setup, signIn } from './test-support/session-backend.js';

const END_SESSION_URL = 'https://identity.example.test/realm/logout?id_token_hint=x';

describe('BrowserSessionClient logout', () => {
  it('forces a fresh login after logout until the next callback succeeds', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = await signIn(tab);
    server.logout = () => jsonResponse({ endSessionUrl: END_SESSION_URL });

    await expect(client.logout()).resolves.toBe(true);
    expect(tab.navigations.at(-1)).toBe(END_SESSION_URL);
    expect(browser.localStorage.getItem('product:force-login')).toBe('1');

    await client.login();
    const authorization = new URL(required(tab.navigations.at(-1)));
    expect(authorization.searchParams.get('prompt')).toBe('login');
    const returned = new BrowserSessionClient(CONFIG, tab.environment);
    tab.setCurrentUrl(
      `https://app.example.test/?code=next&state=${required(authorization.searchParams.get('state'))}`,
    );
    await expect(returned.handleCallback()).resolves.toBe(true);
    expect(browser.localStorage.getItem('product:force-login')).toBeNull();

    await returned.login();
    const plain = new URL(required(tab.navigations.at(-1)));
    expect(plain.searchParams.get('prompt')).toBeNull();
  });

  it('stays on the page when the IdP session is kept, so an account switch can log in', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = await signIn(tab);
    server.logout = () => jsonResponse({ endSessionUrl: END_SESSION_URL });
    const navigationsBefore = tab.navigations.length;

    await expect(client.logout({ endIdpSession: false })).resolves.toBe(false);

    expect(sessionCalls(server, 'logout')).toHaveLength(1);
    expect(tab.navigations).toHaveLength(navigationsBefore);
    expect(client.hasSession()).toBe(false);
    await client.login();
    const authorization = new URL(required(tab.navigations.at(-1)));
    expect(authorization.searchParams.get('prompt')).toBe('login');
  });

  it('keeps the fresh-login mark when the backend logout fails', async () => {
    const { browser } = setup();
    const tab = browser.tab();
    const client = await signIn(tab);
    browser.fetch.mockRejectedValueOnce(new TypeError('offline'));

    await expect(client.logout()).rejects.toMatchObject({ code: 'logout_failed' });
    expect(browser.localStorage.getItem('product:force-login')).toBe('1');
  });
});
