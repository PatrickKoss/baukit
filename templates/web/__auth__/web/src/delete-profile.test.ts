// @vitest-environment jsdom

import { webcrypto } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MockFetch } from '@baukit/api-runtime';

vi.mock('./auth', () => ({
  authClient: {
    accessToken: vi.fn(() => Promise.resolve('token-before-deletion')),
    clearSession: vi.fn(),
  },
}));

import { authClient } from './auth';
import { createDeleteProfileClient, deleteProfile } from './delete-profile';

const operationId = 'bd6f2039-9143-44c4-b41a-43aa5626b7b2';

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal('crypto', webcrypto);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe('delete profile service', () => {
  it.each([200, 202, 401])(
    'removes the durable key after a definitive %i response',
    async (status) => {
      const body =
        status === 401
          ? {
              error: {
                code: 'profile_erased',
                message: 'Erased',
                requestId: 'request-1',
                details: {},
              },
            }
          : {
              status: status === 200 ? 'completed' : 'pending',
              operationId,
              completedAt: '2026-10-03T12:00:00Z',
            };
      const fetch = new MockFetch().enqueueJson(body, { status });
      vi.stubGlobal('fetch', fetch.fetch);
      const client = await createDeleteProfileClient('private-subject');
      const result = await deleteProfile({
        client,
        eraseLocalPartition: () => Promise.resolve(),
        onSignedOut: () => undefined,
      });
      expect(result.status).toBe(status === 200 ? 'erased' : 'pending');
      expect(localStorage.length).toBe(0);
    },
  );

  it('still erases local data and signs out if key removal fails after commit', async () => {
    const fetch = new MockFetch().enqueueJson({ status: 'pending', operationId }, { status: 202 });
    vi.stubGlobal('fetch', fetch.fetch);
    const remove = vi.spyOn(Storage.prototype, 'removeItem').mockImplementationOnce(() => {
      throw new Error('Storage unavailable');
    });
    const eraseLocalPartition = vi.fn(() => Promise.resolve());
    const client = await createDeleteProfileClient('private-subject');
    await expect(
      deleteProfile({
        client,
        eraseLocalPartition,
        onSignedOut: () => undefined,
      }),
    ).resolves.toMatchObject({
      status: 'local-failure',
      receipt: { status: 'pending', operationId },
    });
    expect(eraseLocalPartition).toHaveBeenCalledOnce();
    expect(authClient.clearSession).toHaveBeenCalledOnce();
    remove.mockRestore();
  });

  it('reuses a durable key after a lost response and then clears local data and auth', async () => {
    const fetch = new MockFetch()
      .enqueue(new TypeError('connection lost'))
      .enqueueJson({ status: 'pending', operationId }, { status: 202 });
    vi.stubGlobal('fetch', fetch.fetch);
    const eraseLocalPartition = vi.fn(() => Promise.resolve());
    const onSignedOut = vi.fn();
    const first = await createDeleteProfileClient('private-subject');
    const result = await deleteProfile({
      client: first,
      eraseLocalPartition,
      onSignedOut,
    });
    expect(result.status).toBe('ambiguous');
    expect(eraseLocalPartition).not.toHaveBeenCalled();
    expect(authClient.clearSession).not.toHaveBeenCalled();
    expect(JSON.stringify(localStorage)).not.toContain('private-subject');

    const second = await createDeleteProfileClient('private-subject');
    await expect(
      deleteProfile({ client: second, eraseLocalPartition, onSignedOut }),
    ).resolves.toMatchObject({ status: 'pending', receipt: { operationId } });
    expect(fetch.request(1).headers.get('Idempotency-Key')).toBe(
      fetch.request(0).headers.get('Idempotency-Key'),
    );
    expect(eraseLocalPartition).toHaveBeenCalledOnce();
    expect(authClient.clearSession).toHaveBeenCalledOnce();
    expect(onSignedOut).toHaveBeenCalledOnce();
    expect(localStorage.length).toBe(0);
  });

  it('checks status with the captured token after local sign-out', async () => {
    const fetch = new MockFetch()
      .enqueueJson({ status: 'pending', operationId }, { status: 202 })
      .enqueueJson({ status: 'completed', operationId });
    vi.stubGlobal('fetch', fetch.fetch);
    const client = await createDeleteProfileClient('subject');
    await deleteProfile({
      client,
      eraseLocalPartition: () => Promise.resolve(),
      onSignedOut: () => undefined,
    });
    vi.mocked(authClient.accessToken).mockResolvedValueOnce(undefined);
    await expect(client.poll(operationId)).resolves.toMatchObject({
      status: 'completed',
    });
    expect(fetch.request(1).headers.get('Authorization')).toBe('Bearer token-before-deletion');
    expect(authClient.accessToken).toHaveBeenCalledOnce();
  });
});
