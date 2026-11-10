import { describe, expect, it } from 'vitest';

import { BrowserSessionClient } from './index.js';
import { required } from './test-support/browser-session.js';
import { CONFIG, sessionCalls, setup } from './test-support/session-backend.js';

describe('BrowserSessionClient callback detection', () => {
  it('ignores code and state parameters outside the redirect URI', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);
    await client.login();
    const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
    const elsewhere = `https://app.example.test/suite/link?code=c&state=${state}`;

    tab.setCurrentUrl(elsewhere);

    await expect(client.handleCallback()).resolves.toBe(false);
    expect(sessionCalls(server, 'exchange')).toHaveLength(0);
    tab.setCurrentUrl(`https://app.example.test/?code=c&state=${state}`);
    const reloaded = new BrowserSessionClient(CONFIG, tab.environment);
    await expect(reloaded.handleCallback()).resolves.toBe(true);
    expect(sessionCalls(server, 'exchange')).toHaveLength(1);
  });
});
