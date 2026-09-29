import type { FullConfig } from '@playwright/test';
import { allowKeycloakWebOrigin } from '@baukit/auth-node/keycloak-testing';

import { stack } from './keycloak';

export default async function globalSetup(config: FullConfig): Promise<void> {
  const baseURL = config.projects[0]?.use.baseURL;
  if (baseURL === undefined) {
    throw new Error('The stack config must set a baseURL.');
  }
  await allowKeycloakWebOrigin(stack, new URL(baseURL).origin);
}
