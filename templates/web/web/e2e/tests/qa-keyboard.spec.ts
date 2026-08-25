import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import {
  computedFocusVisual,
  expectFocusStaysInside,
  openRoute,
  resetFocusToBody,
  stubApi,
} from './qa';

test.describe('keyboard', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const overlay of qaConfig.overlays) {
    test(`${overlay.name} takes focus, traps Tab, and restores the trigger`, async ({ page }) => {
      await openRoute(page, overlay.path);
      const trigger = page.getByRole('button', { name: overlay.trigger });
      await trigger.focus();
      await expect(trigger).toBeFocused();
      await page.keyboard.press('Enter');

      const dialog = page.getByRole('dialog', { name: overlay.dialog });
      await expect(dialog).toBeVisible();
      await expect(dialog).toHaveAttribute('aria-modal', 'true');
      await expect(page.getByLabel(overlay.initialFocus)).toBeFocused();
      await expect
        .poll(() => trigger.evaluate((element) => Boolean(element.closest('[inert]'))))
        .toBe(true);

      await resetFocusToBody(page);
      await expectFocusStaysInside(page, dialog, 6);

      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(trigger).toBeFocused();
    });
  }

  for (const route of qaConfig.routes) {
    test(`${route.name} never lands keyboard focus on hidden or inert content`, async ({
      page,
    }) => {
      await openRoute(page, route.path);
      await expect(page.getByRole('heading', { name: route.heading, level: 1 })).toBeVisible();

      const focusableCount = await page.evaluate(() => {
        const selector = [
          'a[href]',
          'button:not([disabled])',
          'input:not([disabled])',
          'select:not([disabled])',
          'textarea:not([disabled])',
          '[role="button"]:not([aria-disabled="true"])',
          '[role="link"]',
          '[role="menuitem"]:not([aria-disabled="true"])',
          '[role="radio"]:not([aria-disabled="true"])',
          '[tabindex]:not([tabindex="-1"])',
        ].join(',');
        return Array.from(document.querySelectorAll<HTMLElement>(selector)).filter(
          (element) =>
            element.tabIndex >= 0 &&
            !element.closest('[aria-hidden="true"], [inert]') &&
            getComputedStyle(element).visibility !== 'hidden',
        ).length;
      });
      const presses = Math.min(Math.max(focusableCount + 2, 12), 60);

      await resetFocusToBody(page);
      const unexpectedFocus: Record<string, unknown>[] = [];
      for (let press = 0; press < presses; press += 1) {
        await page.keyboard.press('Tab');
        const focusState = await page.evaluate(() => {
          const active = document.activeElement;
          return {
            activeTag: active?.tagName ?? null,
            hiddenAncestor: active?.closest('[aria-hidden="true"]')?.tagName ?? null,
            inertAncestor: active?.closest('[inert]')?.tagName ?? null,
          };
        });
        if (focusState.hiddenAncestor !== null || focusState.inertAncestor !== null) {
          unexpectedFocus.push({ press: press + 1, ...focusState });
        }
      }
      expect(unexpectedFocus).toEqual([]);
    });

    test(`${route.name} shows a distinct visible focus ring for keyboard focus`, async ({
      page,
    }) => {
      await openRoute(page, route.path);
      const control = page.getByRole('button', { name: route.focusRingControl });
      await expect(control).toBeVisible();

      await control.click();
      const mouseFocus = await computedFocusVisual(control);

      // Tab forward until the control is reached. Retrying the press inside a
      // poll would restart from the body each attempt and never arrive.
      await resetFocusToBody(page);
      let reached = false;
      for (let press = 0; press < 30 && !reached; press += 1) {
        await page.keyboard.press('Tab');
        reached = await control.evaluate((element) => element === document.activeElement);
      }
      expect(reached, 'keyboard focus never reached the first control').toBe(true);
      const keyboardFocus = await computedFocusVisual(control);

      expect(keyboardFocus).not.toEqual(mouseFocus);
      expect(keyboardFocus.outlineStyle).toBe('solid');
      expect(Number.parseFloat(keyboardFocus.outlineWidth)).toBeGreaterThanOrEqual(3);
      expect(Number.parseFloat(keyboardFocus.outlineOffset)).toBeGreaterThanOrEqual(2);
    });
  }
});
