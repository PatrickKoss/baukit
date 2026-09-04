import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import {
  CONSOLE_WARNING_ALLOWLIST,
  collectUnexpectedConsoleWarnings,
  isAllowedConsoleWarning,
} from './console-warnings';
import { openRoute, stubApi } from './qa';

test('the console warning allowlist uses exact messages with reasons', () => {
  for (const entry of CONSOLE_WARNING_ALLOWLIST) {
    expect(entry.message.length).toBeGreaterThan(0);
    expect(entry.reason.length).toBeGreaterThan(0);
    expect(isAllowedConsoleWarning(entry.message)).toBe(true);
    expect(isAllowedConsoleWarning(`${entry.message} with new detail`)).toBe(false);
  }
});

for (const route of qaConfig.routes) {
  test(`${route.name} emits no unexpected console warnings`, async ({ page }, testInfo) => {
    const unexpected = collectUnexpectedConsoleWarnings(page);
    await stubApi(page, qaConfig.apiStubs);
    await stubApi(page, route.apiStubs ?? []);

    await openRoute(page, route.path, route.authenticated);
    await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();

    await testInfo.attach('console-warnings', {
      body: Buffer.from(JSON.stringify(unexpected, null, 2)),
      contentType: 'application/json',
    });
    await expect.poll(() => unexpected).toEqual([]);
  });
}
