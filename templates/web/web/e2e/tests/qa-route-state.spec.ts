import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, stubApi } from './qa';

/**
 * A deep link that cannot resolve must land in a contextual state with a way
 * out. Rendering nothing, a bare error, or a loading spinner that never settles
 * are all failures.
 */
test.describe('route state', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const routeState of qaConfig.routeStates) {
    test(`${routeState.name} renders a contextual state with a recovery action`, async ({
      page,
    }) => {
      await openRoute(page, routeState.path);

      const heading = page.getByRole('heading', { name: routeState.heading });
      await expect(heading).toBeVisible();
      const recovery = page.getByRole('button', { name: routeState.recovery });
      await expect(recovery).toBeVisible();
      await expect(page.getByText('Loading', { exact: false })).toHaveCount(0);
    });
  }

  for (const route of qaConfig.routes) {
    test(`${route.name} withholds its settled state until the first load resolves`, async ({
      page,
    }) => {
      let release!: () => void;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      for (const stub of qaConfig.apiStubs) {
        await page.unroute(stub.url);
        await page.route(stub.url, async (routeCall) => {
          await gate;
          await routeCall.fulfill({
            status: stub.status,
            contentType: 'application/json',
            body: JSON.stringify(stub.body),
          });
        });
      }

      const navigation = page.goto(route.path);
      await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();
      await expect(page.getByText('Loading', { exact: false }).first()).toBeVisible();

      release();
      await navigation;
      await expect(page.getByText('Loading', { exact: false })).toHaveCount(0);
    });
  }
});
