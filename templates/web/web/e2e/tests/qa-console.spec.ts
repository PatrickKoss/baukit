import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import {
  CONSOLE_WARNING_ALLOWLIST,
  collectContentSecurityPolicyViolations,
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

test('the served policy blocks an injected inline script and the collector sees it', async ({
  page,
}) => {
  const violations = await collectContentSecurityPolicyViolations(page);
  await stubApi(page, qaConfig.apiStubs);

  const response = await page.goto('/');
  const policy = response?.headers()['content-security-policy'] ?? '';
  expect(policy).toContain("script-src 'self'");
  expect(policy).not.toContain("'unsafe-inline'");
  await page.evaluate(() => {
    const script = document.createElement('script');
    script.textContent = 'document.body.dataset.injected = "ran";';
    document.head.append(script);
  });

  await expect.poll(() => violations.some((entry) => entry.includes('script-src'))).toBe(true);
  await expect(page.locator('body')).not.toHaveAttribute('data-injected', 'ran');
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

  test(`${route.name} reports no Content Security Policy violations`, async ({ page }, testInfo) => {
    const violations = await collectContentSecurityPolicyViolations(page);
    await stubApi(page, qaConfig.apiStubs);
    await stubApi(page, route.apiStubs ?? []);

    await openRoute(page, route.path, route.authenticated);
    await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();

    await testInfo.attach('csp-violations', {
      body: Buffer.from(JSON.stringify(violations, null, 2)),
      contentType: 'application/json',
    });
    expect(violations).toEqual([]);
  });
}
