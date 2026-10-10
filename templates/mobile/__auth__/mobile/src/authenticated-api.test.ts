import { describe, expect, it, jest } from '@jest/globals';
import { MockFetch } from '@baukit/api-runtime';

import { type AuthTokenClient, createAuthenticatedApiRuntime } from './authenticated-api';

const unauthorized = {
  error: {
    code: 'unauthenticated',
    message: 'Authentication required',
    requestId: 'request-1',
    details: {},
  },
};

function tokenClient(...tokens: (string | undefined)[]) {
  const accessToken = jest.fn<AuthTokenClient['accessToken']>();
  for (const token of tokens) {
    accessToken.mockResolvedValueOnce(token);
  }
  return accessToken;
}

function runtimeFor(accessToken: AuthTokenClient['accessToken'], fetch?: MockFetch) {
  return createAuthenticatedApiRuntime({
    auth: { accessToken, clearSession: () => undefined },
    baseUrl: 'https://api.example.test',
    environment: 'test',
    ...(fetch === undefined ? {} : { fetch: fetch.fetch }),
  });
}

describe('authenticated API runtime', () => {
  it('refreshes credentials and replays a 401 once', async () => {
    const fetch = new MockFetch()
      .enqueueJson(unauthorized, { status: 401 })
      .enqueueJson({ id: 'user-1', subject: 'subject-1' });
    const accessToken = tokenClient('expired-token', 'fresh-token', 'fresh-token');

    await expect(runtimeFor(accessToken, fetch).fetch('/me')).resolves.toHaveProperty(
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
    const accessToken = tokenClient('expired-token', 'fresh-token', 'fresh-token');

    await expect(runtimeFor(accessToken, fetch).fetch('/me')).rejects.toMatchObject({
      status: 401,
    });

    expect(fetch.requests).toHaveLength(2);
    expect(accessToken).toHaveBeenCalledTimes(3);
  });

  it('sends no bearer token and skips the replay without a session', async () => {
    const fetch = new MockFetch().enqueueJson(unauthorized, { status: 401 });
    const accessToken = tokenClient(undefined, undefined);

    await expect(runtimeFor(accessToken, fetch).fetch('/me')).rejects.toMatchObject({
      status: 401,
    });

    expect(fetch.requests).toHaveLength(1);
    expect(fetch.request(0).headers.get('authorization')).toBeNull();
    expect(accessToken).toHaveBeenLastCalledWith({ forceRefresh: true });
  });

  it('falls back to the global fetch', async () => {
    const fetch = new MockFetch().enqueueJson({ id: 'user-1', subject: 'subject-1' });
    const globalFetch = jest.spyOn(globalThis, 'fetch').mockImplementation(fetch.fetch);

    await expect(runtimeFor(tokenClient('token')).fetch('/me')).resolves.toHaveProperty(
      'status',
      200,
    );

    expect(globalFetch).toHaveBeenCalledTimes(1);
    globalFetch.mockRestore();
  });
});


it('clears an erased profile session and uses the signed-out route without refresh', async () => {
  const fetch = new MockFetch().enqueueJson({
    error: { ...unauthorized.error, code: 'profile_erased' },
  }, { status: 401 });
  const order: string[] = [];
  const clearSession = jest.fn(() => { order.push('clear'); });
  const onProfileErased = jest.fn(() => { order.push('signed-out'); });
  const accessToken = jest.fn<AuthTokenClient['accessToken']>().mockResolvedValue('old-token');
  const runtime = createAuthenticatedApiRuntime({
    auth: { accessToken, clearSession }, onProfileErased,
    baseUrl: 'https://api.example.test', environment: 'test', fetch: fetch.fetch,
  });
  await expect(runtime.fetch('/me')).rejects.toMatchObject({ code: 'profile_erased' });
  expect(clearSession).toHaveBeenCalledTimes(1);
  expect(onProfileErased).toHaveBeenCalledTimes(1);
  expect(order).toEqual(['clear', 'signed-out']);
  expect(accessToken).toHaveBeenCalledTimes(1);
  expect(accessToken).not.toHaveBeenCalledWith({ forceRefresh: true });
  expect(fetch.requests).toHaveLength(1);
});
