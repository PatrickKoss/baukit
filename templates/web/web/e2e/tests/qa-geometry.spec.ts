import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import {
  BREAKPOINT_AUDIT_VIEWPORTS,
  expectInsideScrollContainer,
  expectMinimumInteractiveTargetSize,
  expectNoHorizontalDocumentOverflow,
  expectRectanglesDoNotIntersect,
  setAuditViewport,
} from './geometry';
import { openRoute, stubApi } from './qa';

test.describe('geometry', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== 'desktop-chromium',
      'Breakpoint geometry is checked once on Chromium.',
    );
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const route of qaConfig.routes) {
    test(`${route.name} clears fixed navigation at breakpoint boundaries`, async ({ page }) => {
      await openRoute(page, route.path);
      await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();

      for (const viewport of BREAKPOINT_AUDIT_VIEWPORTS) {
        await test.step(viewport.name, async () => {
          await setAuditViewport(page, viewport);
          await expectNoHorizontalDocumentOverflow(page);
          await expectRectanglesDoNotIntersect(
            page.locator(qaConfig.geometry.overlapTargetSelector),
            page.locator(qaConfig.geometry.fixedNavigationSelector),
          );
          await expectInsideScrollContainer(
            page.locator(qaConfig.geometry.scrollTargetSelector),
            page.locator(qaConfig.geometry.scrollContainerSelector),
          );
          await expectMinimumInteractiveTargetSize(
            page.locator(qaConfig.geometry.interactiveTargetSelector),
            qaConfig.geometry.minimumTargetSize,
          );
        });
      }
    });
  }
});
