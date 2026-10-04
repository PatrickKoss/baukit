import { expect, test } from '@playwright/test';

import { expectInitialDialogFocus } from './qa';

for (const control of [
  {
    name: 'Display name',
    markup: '<label>Display name<input autofocus></label>',
    role: 'textbox',
  },
  {
    name: 'Cancel',
    markup: '<button autofocus>Cancel</button>',
    role: 'button',
  },
] as const) {
  test(`initial dialog focus accepts ${control.role} names`, async ({
    page,
  }) => {
    await page.setContent(
      `<dialog aria-label="Edit profile">${control.markup}</dialog>`,
    );
    await page.locator('dialog').evaluate((dialog: HTMLDialogElement) => {
      dialog.showModal();
    });
    const dialog = page.getByRole('dialog', { name: 'Edit profile' });
    await expect(
      dialog.getByRole(control.role, { name: control.name }),
    ).toBeFocused();
    await expectInitialDialogFocus(dialog, control.name);
  });
}
