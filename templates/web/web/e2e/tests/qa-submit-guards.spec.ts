import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, rapidDoubleClick, stubApi } from './qa';

test.describe('submit guards', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const target of qaConfig.submits) {
    test(`${target.name} produces one result under a rapid double submit`, async ({ page }) => {
      await openRoute(page, target.path);
      if (target.open !== undefined) {
        await page.getByRole('button', { name: target.open }).click();
      }
      await page.getByLabel(target.field).fill(target.value);

      await rapidDoubleClick(page.getByRole('button', { name: target.submit }));

      await expect(page.getByText(target.result, { exact: true })).toHaveCount(1);
    });

    test(`${target.name} keeps entered input after a failed submit`, async ({ page }) => {
      await openRoute(page, target.path);
      if (target.open !== undefined) {
        await page.getByRole('button', { name: target.open }).click();
      }
      const field = page.getByLabel(target.field);
      await field.fill('');
      await page.getByRole('button', { name: target.submit }).click();

      await expect(field).toHaveAttribute('aria-invalid', 'true');
      await expect(field).toBeFocused();
      await expect(page.getByText(target.result, { exact: true })).toHaveCount(0);

      await field.fill(target.value);
      await page.getByRole('button', { name: target.submit }).click();
      await expect(page.getByText(target.result, { exact: true })).toHaveCount(1);
      await expect(field).toHaveValue(target.value);
    });
  }
});
