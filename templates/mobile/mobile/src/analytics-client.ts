import { AnalyticsClient, NoopTransport } from '@baukit/analytics-core';
import type { AnalyticsStorage, EventAllowlist } from '@baukit/analytics-core';

import { PRODUCT_NAME } from './product.js';

export interface ProductEvent {
  readonly name: 'items_viewed';
  readonly properties: { readonly count: number };
}

export const analyticsStoragePrefix = `@baukit/analytics-core:${PRODUCT_NAME}`;

const allowlist = {
  items_viewed: ['count'],
} as const satisfies EventAllowlist<ProductEvent>;

export function createAnalytics(
  uuidFactory: () => string,
  environment = 'development',
  storage?: AnalyticsStorage,
): AnalyticsClient<ProductEvent> {
  return new AnalyticsClient<ProductEvent>({
    context: {
      schema_version: 1,
      app: PRODUCT_NAME,
      app_version: '0.1.0',
      platform: 'mobile',
      environment,
      locale: Intl.DateTimeFormat().resolvedOptions().locale,
    },
    allowlist,
    transport: new NoopTransport<ProductEvent>(),
    uuidFactory,
    storageKeyPrefix: analyticsStoragePrefix,
    ...(storage === undefined ? {} : { storage }),
  });
}
