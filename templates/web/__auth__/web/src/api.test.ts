import { createApiRuntime, MockFetch } from '@baukit/api-runtime';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { currentUser, listItems } from './api';

function runtimeWith(mock: MockFetch) {
  return createApiRuntime({
    baseUrl: 'https://api.example.test',
    environment: 'test',
    fetch: mock.fetch,
    requestIdFactory: () => '00000000-0000-4000-8000-000000000001',
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('listItems', () => {
  it('uses the Baukit runtime transport and parses items', async () => {
    const mock = new MockFetch().enqueueJson([
      { id: '018f0000-0000-7000-8000-000000000001', name: 'First item' },
    ]);

    await expect(listItems(runtimeWith(mock).fetch)).resolves.toEqual([
      { id: '018f0000-0000-7000-8000-000000000001', name: 'First item' },
    ]);
    mock.assertRequest(0, {
      method: 'GET',
      url: 'https://api.example.test/items',
    });
    mock.assertQueueEmpty();
  });

  it.each([
    ['an object', { items: [] }],
    ['a null entry', [null]],
    ['an entry without a name', [{ id: 'item-1' }]],
  ])('rejects %s', async (_label, body) => {
    const mock = new MockFetch().enqueueJson(body);

    await expect(listItems(runtimeWith(mock).fetch)).rejects.toThrow(
      'The API returned an invalid items response.',
    );
  });

  it('defaults to the authenticated runtime without a browser session', async () => {
    const mock = new MockFetch().enqueueJson([]);
    vi.stubGlobal('fetch', mock.fetch);
    vi.resetModules();
    const api = await import('./api');

    await expect(api.listItems()).resolves.toEqual([]);
    expect(new URL(mock.request(0).url).pathname).toBe('/items');
    expect(mock.request(0).headers.has('authorization')).toBe(false);
  });
});

describe('currentUser', () => {
  it('parses the authenticated user', async () => {
    const mock = new MockFetch().enqueueJson({ id: 'user-1', subject: 'subject-1' });

    await expect(currentUser(runtimeWith(mock).fetch)).resolves.toEqual({
      id: 'user-1',
      subject: 'subject-1',
    });
    mock.assertRequest(0, { method: 'GET', url: 'https://api.example.test/me' });
  });

  it.each([
    ['null', null],
    ['a user without a subject', { id: 'user-1' }],
  ])('rejects %s', async (_label, body) => {
    const mock = new MockFetch().enqueueJson(body);

    await expect(currentUser(runtimeWith(mock).fetch)).rejects.toThrow(
      'The API returned an invalid current-user response.',
    );
  });

  it('defaults to the authenticated runtime', async () => {
    const mock = new MockFetch().enqueueJson({ id: 'user-1', subject: 'subject-1' });
    vi.stubGlobal('fetch', mock.fetch);
    vi.resetModules();
    const api = await import('./api');

    await expect(api.currentUser()).resolves.toMatchObject({ subject: 'subject-1' });
  });
});
