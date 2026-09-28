import { type FullConfig, request } from '@playwright/test';

import { allowWebOrigin } from './keycloak';

export default async function globalSetup(config: FullConfig): Promise<void> {
  const baseURL = config.projects[0]?.use.baseURL;
  if (baseURL === undefined) {
    throw new Error('The stack config must set a baseURL.');
  }
  const context = await request.newContext();
  try {
    await allowWebOrigin(context, new URL(baseURL).origin);
  } finally {
    await context.dispose();
  }
}
