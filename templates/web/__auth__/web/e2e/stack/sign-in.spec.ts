import { expect, test } from '@playwright/test';

import { createKeycloakTestUser, signInWithKeycloak } from './keycloak';

test('a new Keycloak user signs in and sees their subject', async ({ page, request }) => {
  const user = await createKeycloakTestUser(request);

  await page.goto('/');
  await page.getByRole('button', { name: 'Sign in with local Keycloak' }).click();
  await signInWithKeycloak(page, user);

  await expect(page.getByText(`Signed in as ${user.subject}`)).toBeVisible();
});
