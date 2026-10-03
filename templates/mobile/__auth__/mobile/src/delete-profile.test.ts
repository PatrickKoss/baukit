import { MockFetch } from '@baukit/api-runtime';

const mockKeys = new Map<string, string>();
const mockReadToken = jest.fn(() => Promise.resolve('token-before-deletion'));
const mockClearSession = jest.fn(() => Promise.resolve());
const mockDeletePreference = jest.fn(() => Promise.resolve());
const mockCloseDatabase = jest.fn(() => Promise.resolve());

jest.mock('./auth', () => ({
  authClient: {
    accessToken: () => mockReadToken(),
    clearSession: () => mockClearSession(),
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
  getItemAsync: (key: string) => Promise.resolve(mockKeys.get(key) ?? null),
  setItemAsync: (key: string, value: string) => {
    mockKeys.set(key, value);
    return Promise.resolve();
  },
  deleteItemAsync: (key: string) => {
    mockKeys.delete(key);
    return Promise.resolve();
  },
}));
jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: () => Promise.resolve({ closeAsync: mockCloseDatabase }),
}));
jest.mock('./record-store', () => ({
  createAppPreferenceRecordStore: () => Promise.resolve({ delete: mockDeletePreference }),
}));

import { createDeleteProfileClient, deleteProfile } from './delete-profile';

const operationId = 'bd6f2039-9143-44c4-b41a-43aa5626b7b2';
let previousFetch: typeof globalThis.fetch;

beforeEach(() => {
  mockKeys.clear();
  previousFetch = globalThis.fetch;
});
afterEach(() => {
  globalThis.fetch = previousFetch;
});

function dependencies(client: Awaited<ReturnType<typeof createDeleteProfileClient>>) {
  return {
    client,
    subject: 'private-subject',
    eraseLocalPartition: jest.fn(() => Promise.resolve()),
    resetPreferenceIdentity: jest.fn(() => Promise.resolve()),
  };
}

describe('delete profile service', () => {
  it('replays after a dropped response, deletes only this account preferences and signs out', async () => {
    const fetch = new MockFetch()
      .enqueue(new TypeError('connection lost'))
      .enqueueJson({ status: 'pending', operationId }, { status: 202 });
    globalThis.fetch = fetch.fetch;
    const first = dependencies(await createDeleteProfileClient('private-subject'));
    expect((await deleteProfile(first)).status).toBe('ambiguous');
    expect(first.eraseLocalPartition).not.toHaveBeenCalled();
    expect(mockClearSession).not.toHaveBeenCalled();
    expect(JSON.stringify([...mockKeys])).not.toContain('private-subject');

    const second = dependencies(await createDeleteProfileClient('private-subject'));
    await expect(deleteProfile(second)).resolves.toMatchObject({
      status: 'pending',
      receipt: { operationId },
    });
    expect(fetch.request(1).headers.get('Idempotency-Key')).toBe(
      fetch.request(0).headers.get('Idempotency-Key'),
    );
    expect(second.eraseLocalPartition).toHaveBeenCalledTimes(1);
    expect(second.resetPreferenceIdentity).toHaveBeenCalledTimes(1);
    expect(mockDeletePreference).toHaveBeenCalledWith('private-subject');
    expect(mockCloseDatabase).toHaveBeenCalledTimes(1);
    expect(mockClearSession).toHaveBeenCalledTimes(1);
  });

  it('signs out even if partition and preference deletion fail', async () => {
    const fetch = new MockFetch().enqueueJson({ status: 'pending', operationId }, { status: 202 });
    globalThis.fetch = fetch.fetch;
    const deps = dependencies(await createDeleteProfileClient('private-subject'));
    deps.eraseLocalPartition.mockRejectedValueOnce(new Error('disk error'));
    mockDeletePreference.mockRejectedValueOnce(new Error('preference disk error'));
    await expect(deleteProfile(deps)).resolves.toMatchObject({
      status: 'local-failure',
      error: { stage: 'local', cause: 'AggregateError' },
    });
    expect(mockCloseDatabase).toHaveBeenCalledTimes(1);
    expect(mockClearSession).toHaveBeenCalledTimes(1);
  });

  it('still attempts both local deletions if resetting visible preferences fails', async () => {
    const fetch = new MockFetch().enqueueJson({ status: 'pending', operationId }, { status: 202 });
    globalThis.fetch = fetch.fetch;
    const deps = dependencies(await createDeleteProfileClient('private-subject'));
    deps.resetPreferenceIdentity.mockRejectedValueOnce(new Error('preference reset failed'));
    await expect(deleteProfile(deps)).resolves.toMatchObject({
      status: 'local-failure',
    });
    expect(deps.eraseLocalPartition).toHaveBeenCalledTimes(1);
    expect(mockDeletePreference).toHaveBeenCalledWith('private-subject');
    expect(mockClearSession).toHaveBeenCalledTimes(1);
  });

  it('reports sign-out failures after local deletion and checks status with the captured token', async () => {
    const fetch = new MockFetch()
      .enqueueJson({ status: 'pending', operationId }, { status: 202 })
      .enqueueJson({ status: 'completed', operationId });
    globalThis.fetch = fetch.fetch;
    const deps = dependencies(await createDeleteProfileClient('private-subject'));
    mockClearSession.mockRejectedValueOnce(new Error('token deletion failed'));
    await expect(deleteProfile(deps)).resolves.toMatchObject({
      status: 'signout-failure',
    });
    await expect(deps.client.poll(operationId)).resolves.toMatchObject({
      status: 'completed',
    });
    expect(fetch.request(1).headers.get('Authorization')).toBe('Bearer token-before-deletion');
    expect(mockReadToken).toHaveBeenCalledTimes(1);
  });
});
