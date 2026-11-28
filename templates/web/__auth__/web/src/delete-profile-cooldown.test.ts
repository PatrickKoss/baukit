// @vitest-environment jsdom

import { webcrypto } from 'node:crypto';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { isApiUnavailableError, MockFetch } from '@baukit/api-runtime';

vi.mock('./auth', () => ({
  authClient: {
    accessToken: vi.fn(() => Promise.resolve('token-before-deletion')),
    clearSession: vi.fn(),
  },
}));

import { listItems } from './api';
import { apiAvailability } from './api-availability';
import { createDeleteProfileClient } from './delete-profile';

const RETRY_AFTER_SECONDS = 5;

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal('crypto', webcrypto);
});
afterEach(() => {
  vi.unstubAllGlobals();
  apiAvailability.markAvailable();
});

it('puts a delete 503 with Retry-After into the app-wide cooldown and the banner', async () => {
  const before = Date.now();
  const fetch = new MockFetch().enqueue(
    new Response(null, { status: 503, headers: { 'retry-after': String(RETRY_AFTER_SECONDS) } }),
  );
  vi.stubGlobal('fetch', fetch.fetch);
  const client = await createDeleteProfileClient('private-subject');

  await expect(client.erase()).rejects.toBeDefined();

  expect(apiAvailability.retryAt()).toBeGreaterThanOrEqual(before + RETRY_AFTER_SECONDS * 1_000);
  const appCall = await listItems().catch((cause: unknown) => cause);
  expect(isApiUnavailableError(appCall)).toBe(true);
  expect(fetch.requests).toHaveLength(1);
});
