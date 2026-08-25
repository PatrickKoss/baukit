import { describe, expect, it } from '@jest/globals';

import { createAnalytics } from './analytics-client';

describe('analytics consent', () => {
  it('starts unknown and applies explicit choices', async () => {
    const analytics = createAnalytics(() => '00000000-0000-4000-8000-000000000001');

    expect(analytics.consent).toBe('unknown');
    analytics.capture({
      name: 'items_viewed',
      properties: { count: 1 },
    });
    expect(analytics.pendingCount).toBe(0);

    analytics.setConsent('granted');
    analytics.capture({
      name: 'items_viewed',
      properties: { count: 1 },
    });
    expect(analytics.pendingCount).toBe(1);

    analytics.setConsent('denied');
    expect(analytics.pendingCount).toBe(0);
    await analytics.dispose();
  });
});
