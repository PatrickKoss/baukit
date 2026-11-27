import { isApiUnavailableError, MockFetch } from '@baukit/api-runtime';

jest.mock('./auth', () => ({
  authClient: {
    accessToken: () => Promise.resolve('token-before-deletion'),
    clearSession: () => Promise.resolve(),
  },
}));
jest.mock('expo-crypto', () => {
  const { createHash, randomUUID } =
    jest.requireActual<typeof import('node:crypto')>('node:crypto');
  return {
    CryptoDigestAlgorithm: { SHA256: 'SHA-256' },
    digestStringAsync: (_algorithm: string, text: string) =>
      Promise.resolve(createHash('sha256').update(text).digest('hex')),
    randomUUID,
  };
});
jest.mock('expo-secure-store', () => ({
  getItemAsync: () => Promise.resolve(null),
  setItemAsync: () => Promise.resolve(),
  deleteItemAsync: () => Promise.resolve(),
}));
jest.mock('expo-sqlite', () => ({ openDatabaseAsync: jest.fn() }));
jest.mock('./record-store', () => ({ createAppPreferenceRecordStore: jest.fn() }));

import { listItems } from './api';
import { apiAvailability } from './api-availability';
import { createDeleteProfileClient } from './delete-profile';

const RETRY_AFTER_SECONDS = 5;
let previousFetch: typeof globalThis.fetch;

beforeEach(() => {
  previousFetch = globalThis.fetch;
});
afterEach(() => {
  globalThis.fetch = previousFetch;
  apiAvailability.markAvailable();
});

it('puts a delete 503 with Retry-After into the app-wide cooldown and the banner', async () => {
  const before = Date.now();
  const fetch = new MockFetch().enqueue(
    new Response(null, { status: 503, headers: { 'retry-after': String(RETRY_AFTER_SECONDS) } }),
  );
  globalThis.fetch = fetch.fetch;
  const client = await createDeleteProfileClient('private-subject');

  await expect(client.erase()).rejects.toBeDefined();

  expect(apiAvailability.retryAt()).toBeGreaterThanOrEqual(before + RETRY_AFTER_SECONDS * 1_000);
  const appCall = await listItems().catch((cause: unknown) => cause);
  expect(isApiUnavailableError(appCall)).toBe(true);
  expect(fetch.requests).toHaveLength(1);
});
