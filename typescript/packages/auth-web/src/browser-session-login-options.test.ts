import { describe, expect, it } from 'vitest';

import { appearanceStateDecoration, BrowserSessionClient } from './index.js';
import { required } from './test-support/browser-session.js';
import { CONFIG, sessionCalls, setup } from './test-support/session-backend.js';

describe('BrowserSessionClient login options', () => {
  it('sends the decorated state and the registration prompt to the provider', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);

    await client.login({
      stateDecoration: appearanceStateDecoration({ mode: 'dark' }),
      entry: 'register',
    });

    const authorization = new URL(required(tab.navigations.at(-1)));
    const state = required(authorization.searchParams.get('state'));
    expect(authorization.searchParams.get('prompt')).toBe('create');
    expect(state).toMatch(/^ap1\.d\.[0-9a-f]{64}$/u);
    tab.setCurrentUrl(`https://app.example.test/?code=c&state=${state}`);
    await expect(client.handleCallback()).resolves.toBe(true);
    expect(sessionCalls(server, 'exchange')).toHaveLength(1);
  });

  it('rejects a callback whose decorated state was altered', async () => {
    const { browser } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);
    await client.login({ stateDecoration: appearanceStateDecoration({ mode: 'light' }) });
    const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));

    tab.setCurrentUrl(`https://app.example.test/?code=c&state=${state.replace('.l.', '.d.')}`);

    await expect(client.handleCallback()).rejects.toMatchObject({
      code: 'callback_state_mismatch',
    });
  });
});
