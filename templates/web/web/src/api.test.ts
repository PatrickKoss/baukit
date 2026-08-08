import { createApiRuntime, MockFetch } from '@baukit/api-runtime';
import { describe, expect, it } from 'vitest';

import { listItems } from './api';

describe('listItems', () => {
  it('uses the Baukit runtime transport and parses items', async () => {
    const mock = new MockFetch().enqueueJson([
      { id: '018f0000-0000-7000-8000-000000000001', name: 'First item' },
    ]);
    const runtime = createApiRuntime({
      baseUrl: 'https://api.example.test',
      environment: 'test',
      fetch: mock.fetch,
      requestIdFactory: () => '00000000-0000-4000-8000-000000000001',
    });

    await expect(listItems(runtime.fetch)).resolves.toEqual([
      { id: '018f0000-0000-7000-8000-000000000001', name: 'First item' },
    ]);
    mock.assertRequest(0, {
      method: 'GET',
      url: 'https://api.example.test/items',
    });
    mock.assertQueueEmpty();
  });
});
