import { AnalyticsClient, NoopTransport } from '@baukit/analytics-core';
import type { AnalyticsStorage, EventAllowlist } from '@baukit/analytics-core';

export interface ProductEvent {
  readonly name: 'items_viewed';
  readonly properties: { readonly count: number };
}

const allowlist = {
  items_viewed: ['count'],
} as const satisfies EventAllowlist<ProductEvent>;

const storage: AnalyticsStorage = {
  getItem(key) {
    try {
      return localStorage.getItem(key) ?? undefined;
    } catch {
      return undefined;
    }
  },
  setItem(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Analytics persistence failures remain privacy-safe and non-blocking.
    }
  },
  removeItem(key) {
    try {
      localStorage.removeItem(key);
    } catch {
      // Analytics persistence failures remain privacy-safe and non-blocking.
    }
  },
};

export const analytics = new AnalyticsClient<ProductEvent>({
  context: {
    schema_version: 1,
    app: '{{ context.app_name }}',
    app_version: '0.1.0',
    platform: 'web',
    environment: import.meta.env.MODE,
    locale: navigator.language,
  },
  allowlist,
  storage,
  transport: new NoopTransport<ProductEvent>(),
});
