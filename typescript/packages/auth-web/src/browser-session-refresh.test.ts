import { describe, expect, it, vi } from 'vitest';

import { BrowserSessionClient, SESSION_HANDOFF_TIMEOUT_MS } from './browser-session-client.js';
import { ChannelHub } from './test-support/browser-session.js';
import {
  CONFIG,
  ROTATED_TOKEN,
  sessionCalls,
  setup,
  signIn,
  storageEntries,
} from './test-support/session-backend.js';

describe('BrowserSessionClient refresh handoff', () => {
  it("refreshes once when the winning tab's session message arrives after the lock is released", async () => {
    const { browser, server } = setup();
    await signIn(browser.tab());
    const lateChannel = new ChannelHub('task');
    const tabs = [browser.tab(), browser.tab()].map(
      (tab) =>
        new BrowserSessionClient(CONFIG, { ...tab.environment, openChannel: lateChannel.open }),
    );

    await expect(Promise.all(tabs.map((client) => client.getAccessToken()))).resolves.toEqual([
      ROTATED_TOKEN,
      ROTATED_TOKEN,
    ]);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    expect(JSON.stringify(Object.fromEntries(storageEntries(browser.localStorage)))).not.toMatch(
      /eyJ|expiresAt/u,
    );
  });

  it("refreshes after the handoff timeout when the winning tab's message never arrives", async () => {
    vi.useFakeTimers();
    try {
      const { browser, server } = setup();
      await signIn(browser.tab());
      const silent = {
        postMessage: () => undefined,
        subscribe: () => undefined,
        close: () => undefined,
      };
      const tabs = [browser.tab(), browser.tab()].map(
        (tab) =>
          new BrowserSessionClient(CONFIG, { ...tab.environment, openChannel: () => silent }),
      );

      const pending = Promise.all(tabs.map((client) => client.getAccessToken()));
      await vi.advanceTimersByTimeAsync(SESSION_HANDOFF_TIMEOUT_MS);

      await expect(pending).resolves.toEqual([ROTATED_TOKEN, ROTATED_TOKEN]);
      expect(sessionCalls(server, 'refresh')).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
