import { describe, expect, it, vi } from 'vitest';

import { MockFetch } from '@baukit/api-runtime';

import { createAuthenticatedApiRuntime, type AuthTokenClient } from './authenticated-api';

const unauthorized = {
  error: {
    code: 'unauthenticated',
    message: 'Authentication required',
    requestId: 'request-1',
    details: {},
  },
};

function runtimeWith(accessToken: AuthTokenClient['accessToken'], fetch: MockFetch) {
  return createAuthenticatedApiRuntime({
    auth: { accessToken },
    baseUrl: 'https://api.example.test',
    environment: 'test',
    fetch: fetch.fetch,
  });
}

describe('createAuthenticatedApiRuntime', () => {
  it('refreshes credentials and replays a 401 once', async () => {
    const fetch = new MockFetch()
      .enqueueJson(unauthorized, { status: 401 })
      .enqueueJson({ id: 'user-1', subject: 'subject-1' });
    const accessToken = vi
      .fn<AuthTokenClient['accessToken']>()
      .mockResolvedValueOnce('expired-token')
      .mockResolvedValue('fresh-token');

    await expect(runtimeWith(accessToken, fetch).fetch('/me')).resolves.toHaveProperty(
      'status',
      200,
    );
    expect(fetch.requests).toHaveLength(2);
    expect(fetch.request(0).headers.get('authorization')).toBe('Bearer expired-token');
    expect(fetch.request(1).headers.get('authorization')).toBe('Bearer fresh-token');
    expect(accessToken).toHaveBeenNthCalledWith(2, { forceRefresh: true });
  });

  it('stops after one replay when the refreshed request is also unauthorized', async () => {
    const fetch = new MockFetch()
      .enqueueJson(unauthorized, { status: 401 })
      .enqueueJson(unauthorized, { status: 401 });
    const accessToken = vi
      .fn<AuthTokenClient['accessToken']>()
      .mockResolvedValueOnce('expired-token')
      .mockResolvedValue('fresh-token');

    await expect(runtimeWith(accessToken, fetch).fetch('/me')).rejects.toMatchObject({
      status: 401,
    });
    expect(fetch.requests).toHaveLength(2);
    expect(accessToken).toHaveBeenCalledTimes(3);
  });

  it('sends no credentials and does not replay without a session', async () => {
    const fetch = new MockFetch().enqueueJson(unauthorized, { status: 401 });
    const accessToken = vi.fn<AuthTokenClient['accessToken']>().mockResolvedValue(undefined);

    await expect(runtimeWith(accessToken, fetch).fetch('/me')).rejects.toMatchObject({
      status: 401,
    });
    expect(fetch.requests).toHaveLength(1);
    expect(fetch.request(0).headers.has('authorization')).toBe(false);
    expect(accessToken).toHaveBeenLastCalledWith({ forceRefresh: true });
  });

  it('uses the global fetch when none is injected', async () => {
    const fetch = new MockFetch().enqueueJson({ id: 'user-1', subject: 'subject-1' });
    vi.stubGlobal('fetch', fetch.fetch);
    try {
      const runtime = createAuthenticatedApiRuntime({
        auth: { accessToken: () => Promise.resolve('token') },
        baseUrl: 'https://api.example.test',
        environment: 'test',
      });

      await expect(runtime.fetch('/me')).resolves.toHaveProperty('status', 200);
      expect(fetch.request(0).url).toBe('https://api.example.test/me');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
