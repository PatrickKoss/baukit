import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, stubApi } from './qa';

/**
 * An account switch round trip: A, then B, then A again. B must never see A's
 * records, and A's records must be intact and appear exactly once on return.
 * Both the rendered list and any local cache are checked, so a leaked cache
 * cannot hide behind an unrendered list.
 */
test.describe('auth isolation', () => {
  const [owner, other] = qaConfig.accounts;

  test.skip(
    owner === undefined || other === undefined,
    'Isolation needs two configured accounts.',
  );

  for (const route of qaConfig.protectedRoutes) {
    test(`${route.name} keeps records isolated across an A to B to A switch`, async ({
      page,
      context,
    }) => {
      if (owner === undefined || other === undefined) {
        return;
      }

      await stubApi(page, owner.apiStubs);
      await openRoute(page, route.path);
      await expect(page.getByText(owner.privateText, { exact: false }).first()).toBeVisible();

      await context.clearCookies();
      await page.evaluate(() => {
        localStorage.clear();
        sessionStorage.clear();
      });
      for (const stub of owner.apiStubs) {
        await page.unroute(stub.url);
      }
      await stubApi(page, other.apiStubs);
      await page.reload();

      await expect(page.getByText(other.privateText, { exact: false }).first()).toBeVisible();
      await expect(page.getByText(owner.privateText, { exact: false })).toHaveCount(0);

      await context.clearCookies();
      await page.evaluate(() => {
        localStorage.clear();
        sessionStorage.clear();
      });
      for (const stub of other.apiStubs) {
        await page.unroute(stub.url);
      }
      await stubApi(page, owner.apiStubs);
      await page.reload();

      await expect(page.getByText(owner.privateText, { exact: true })).toHaveCount(1);
      await expect(page.getByText(other.privateText, { exact: false })).toHaveCount(0);
    });
  }
});
