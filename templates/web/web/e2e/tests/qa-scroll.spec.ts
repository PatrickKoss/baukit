import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, scrollingAncestorBox, stubApi } from './qa';

const VIEWPORTS = [
  { width: 1280, height: 720 },
  { width: 390, height: 844 },
];

/**
 * A padded inner scroller strands the scrollbar away from the screen edge and
 * makes the bottom of a long route unreachable on small viewports. Both checks
 * catch that.
 */
test.describe('scroll', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const route of qaConfig.routes.filter(({ checkScroll }) => checkScroll !== false)) {
    test(`${route.name} reaches its last content at every viewport`, async ({ page }) => {
      await stubApi(page, route.apiStubs ?? []);
      await openRoute(page, route.path, route.authenticated);
      await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();

      for (const [index, viewport] of VIEWPORTS.entries()) {
        await test.step(`${String(viewport.width)}x${String(viewport.height)}`, async () => {
          await page.setViewportSize(viewport);
          if (index > 0) {
            await page.reload();
            await expect(
              page.getByRole('heading', { name: route.heading, level: 1 }),
            ).toBeVisible();
          }

          const screen = page.locator(route.screenSelector ?? qaConfig.screenSelector).first();
          const last = screen.locator('> :visible').last();
          await expect(async () => {
            await last.scrollIntoViewIfNeeded();
            await expect(last).toBeInViewport();
          }).toPass({ timeout: 20_000 });
        });
      }
    });

    test(`${route.name} uses an edge-aligned scroller`, async ({ page }, testInfo) => {
      test.skip(
        testInfo.project.name !== 'desktop-chromium',
        'Scroller geometry is checked once, on the desktop reference project.',
      );

      await page.setViewportSize({ width: 1440, height: 900 });
      await stubApi(page, route.apiStubs ?? []);
      await openRoute(page, route.path, route.authenticated);
      const screen = page.locator(route.screenSelector ?? qaConfig.screenSelector).first();
      await expect(screen).toBeVisible();

      const scroller = await scrollingAncestorBox(screen);
      const screenBox = await screen.boundingBox();
      expect(screenBox).not.toBeNull();
      if (screenBox === null) {
        return;
      }

      // The screen must sit inside the scroller and stay horizontally centred
      // in it, so no content is clipped and no scrollbar is stranded.
      expect(screenBox.x).toBeGreaterThanOrEqual(scroller.x - 1);
      expect(screenBox.x + screenBox.width).toBeLessThanOrEqual(scroller.x + scroller.width + 1);
      expect(screenBox.x + screenBox.width / 2).toBeCloseTo(scroller.x + scroller.width / 2, 0);
    });
  }
});
