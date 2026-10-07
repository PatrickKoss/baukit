import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, scrollingLayout, stubApi } from './qa';

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

  test('scroller geometry stays consistent while its width changes', async ({ page }) => {
    await page.setContent(`
      <style>
        #scroller { overflow-y: auto; width: 400px; }
        #screen { width: 50%; margin-inline: auto; height: 1000px; }
      </style>
      <div id="scroller"><div id="screen"></div></div>
    `);
    for (const width of [400, 500, 600, 700]) {
      await page.locator('#scroller').evaluate((element, value) => {
        element.style.width = `${String(value)}px`;
      }, width);
      const { screen, scroller } = await scrollingLayout(page.locator('#screen'));
      expect(scroller.width).toBeCloseTo(width, 3);
      expect(screen.width).toBeCloseTo(width / 2, 3);
      expect(screen.x + screen.width / 2).toBeCloseTo(scroller.x + scroller.width / 2, 3);
    }
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

      const { scroller, screen: screenBox } = await scrollingLayout(screen);

      // The screen must sit inside the scroller and stay horizontally centred
      // in it, so no content is clipped and no scrollbar is stranded.
      expect(screenBox.x).toBeGreaterThanOrEqual(scroller.x - 1);
      expect(screenBox.x + screenBox.width).toBeLessThanOrEqual(scroller.x + scroller.width + 1);
      expect(screenBox.x + screenBox.width / 2).toBeCloseTo(scroller.x + scroller.width / 2, 0);
    });
  }
});
