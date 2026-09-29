import { expect, test } from '@playwright/test';
import { createKeycloakTestUser, signInWithKeycloak } from '@baukit/auth-node/keycloak-testing';

import { stack } from './keycloak';

test('a new Keycloak user signs in and sees their subject', async ({ page }) => {
  const user = await createKeycloakTestUser(stack);

  await page.goto('/');
  await page.getByRole('button', { name: 'Sign in with local Keycloak' }).click();
  await signInWithKeycloak(page, user, stack);

  await expect(page.getByText(`Signed in as ${user.subject}`)).toBeVisible();
});
