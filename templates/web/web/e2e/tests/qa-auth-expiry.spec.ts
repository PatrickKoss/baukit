import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { expireSession, openRoute, stubApi } from './qa';

/**
 * When the session ends, private data must leave the screen and the app must
 * say so. Silently showing a stale snapshot as if it were still authoritative
 * is the failure this guards against.
 */
test.describe('auth expiry', () => {
  for (const route of qaConfig.protectedRoutes) {
    test(`${route.name} surfaces an expired session instead of stale private data`, async ({
      page,
    }) => {
      await stubApi(page, qaConfig.apiStubs);
      await openRoute(page, route.path);
      await expect(page.getByText(route.privateText, { exact: false }).first()).toBeVisible();

      await expireSession(page, route.api);
      await page.reload();

      await expect(page.getByText(route.expiredMessage, { exact: false }).first()).toBeVisible();
      await expect(page.getByText(route.privateText, { exact: false })).toHaveCount(0);
    });

    test(`${route.name} recovers once the session is valid again`, async ({ page }) => {
      await expireSession(page, route.api);
      await openRoute(page, route.path);
      await expect(page.getByText(route.expiredMessage, { exact: false }).first()).toBeVisible();

      await page.unroute(route.api);
      await stubApi(page, qaConfig.apiStubs);
      await page.reload();

      await expect(page.getByText(route.privateText, { exact: false }).first()).toBeVisible();
      await expect(page.getByText(route.expiredMessage, { exact: false })).toHaveCount(0);
    });
  }
});
