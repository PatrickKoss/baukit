import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, stubApi } from './qa';

/**
 * Every overlay must be dismissible without committing anything, and dismissing
 * must leave the page it was opened from intact. `dismissesOnScrimClick`
 * declares whether an outside click is one of those affordances; a confirmation
 * dialog that deliberately requires an explicit choice sets it to false.
 */
test.describe('overlay dismissal', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const overlay of qaConfig.overlays) {
    test(`${overlay.name} closes on Escape without committing`, async ({ page }) => {
      await openRoute(page, overlay.path);
      const trigger = page.getByRole('button', { name: overlay.trigger });
      await trigger.click();

      const dialog = page.getByRole('dialog', { name: overlay.dialog });
      await expect(dialog).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(trigger).toBeVisible();
    });

    test(`${overlay.name} closes through its dismiss control`, async ({ page }) => {
      await openRoute(page, overlay.path);
      const trigger = page.getByRole('button', { name: overlay.trigger });
      await trigger.click();

      const dialog = page.getByRole('dialog', { name: overlay.dialog });
      await expect(dialog).toBeVisible();
      await dialog.getByRole('button', { name: overlay.dismiss }).click();
      await expect(dialog).toHaveCount(0);
      await expect(trigger).toBeVisible();
    });

    if (overlay.scrimTestId !== undefined) {
      const scrimTestId = overlay.scrimTestId;
      test(`${overlay.name} closes on an outside click`, async ({ page }) => {
        await openRoute(page, overlay.path);
        const trigger = page.getByRole('button', { name: overlay.trigger });
        await trigger.click();

        const dialog = page.getByRole('dialog', { name: overlay.dialog });
        const scrim = page.getByTestId(scrimTestId);
        await expect(dialog).toBeVisible();
        await expect(scrim).toBeVisible();

        // The top-left corner is outside a panel that is centered on desktop and
        // bottom-anchored on mobile.
        await scrim.click({ position: { x: 5, y: 5 } });
        await expect(dialog).toHaveCount(0);
        await expect(trigger).toBeVisible();
      });
    }
  }
});
