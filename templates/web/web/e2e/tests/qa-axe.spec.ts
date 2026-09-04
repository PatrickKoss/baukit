import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { expectNoBlockingAxeViolations, openRoute, stubApi } from './qa';

test.describe('axe', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const route of qaConfig.routes) {
    test(`${route.name} has no serious or critical axe violations`, async ({ page }, testInfo) => {
      await stubApi(page, route.apiStubs ?? []);
      await openRoute(page, route.path, route.authenticated);
      await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();
      await expectNoBlockingAxeViolations(page, testInfo, route.name);
    });
  }

  for (const overlay of qaConfig.overlays) {
    test(`${overlay.name} has no serious or critical axe violations`, async ({
      page,
    }, testInfo) => {
      await stubApi(page, overlay.apiStubs ?? []);
      await openRoute(page, overlay.path, overlay.authenticated);
      await page.getByRole('button', { name: overlay.trigger }).click();
      await expect(page.getByRole('dialog', { name: overlay.dialog })).toBeVisible();
      await expectNoBlockingAxeViolations(page, testInfo, overlay.name);
    });
  }

  for (const routeState of qaConfig.routeStates) {
    test(`${routeState.name} has no serious or critical axe violations`, async ({
      page,
    }, testInfo) => {
      await stubApi(page, routeState.apiStubs ?? []);
      await openRoute(page, routeState.path, routeState.authenticated);
      await expect(page.getByRole('heading', { name: routeState.heading })).toBeVisible();
      await expectNoBlockingAxeViolations(page, testInfo, routeState.name);
    });
  }
});
