import { expect, test } from '@playwright/test';

import { qaConfig } from '../qa.config';
import { openRoute, rapidDoubleClick, stubApi } from './qa';

test.describe('submit guards', () => {
  test.beforeEach(async ({ page }) => {
    await stubApi(page, qaConfig.apiStubs);
  });

  for (const target of qaConfig.submits) {
    test(`${target.name} produces one result under a rapid double submit`, async ({ page }) => {
      await stubApi(page, target.apiStubs ?? []);
      await openRoute(page, target.path, target.authenticated);
      if (target.open !== undefined) {
        await page.getByRole('button', { name: target.open }).click();
      }
      for (const field of target.fields) {
        await page.getByLabel(field.field, { exact: true }).fill(field.value);
      }

      await rapidDoubleClick(page.getByRole('button', { name: target.submit }));

      await expect(page.getByText(target.result, { exact: true })).toHaveCount(1);
    });

    test(`${target.name} keeps entered input after a failed submit`, async ({ page }) => {
      await stubApi(page, target.apiStubs ?? []);
      await openRoute(page, target.path, target.authenticated);
      if (target.open !== undefined) {
        await page.getByRole('button', { name: target.open }).click();
      }
      for (const item of target.fields) {
        await page.getByLabel(item.field, { exact: true }).fill(item.value);
      }
      const invalidField = target.invalidField;
      const invalidValue = target.fields.find(({ field }) => field === invalidField)?.value;
      if (invalidValue === undefined) {
        throw new Error(`Submit target ${target.name} has no value for ${invalidField}.`);
      }
      const field = page.getByLabel(invalidField, { exact: true });
      await field.fill('');
      await page.getByRole('button', { name: target.submit }).click();

      await expect(field).toHaveAttribute('aria-invalid', 'true');
      await expect(field).toBeFocused();
      await expect(page.getByText(target.result, { exact: true })).toHaveCount(0);

      await field.fill(invalidValue);
      await page.getByRole('button', { name: target.submit }).click();
      await expect(page.getByText(target.result, { exact: true })).toHaveCount(1);
      for (const item of target.fields) {
        await expect(page.getByLabel(item.field, { exact: true })).toHaveValue(item.value);
      }
    });
  }
});
