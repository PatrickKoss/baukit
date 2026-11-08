import { describe, expect, it } from '@jest/globals';

import { browserSignInClient, type BrowserProviderClient } from './browser-client';

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `header.${encoded.replace(/=+$/u, '')}.signature`;
}

const token = unsignedJwt({ sub: 'user_1', exp: Math.floor(Date.now() / 1000) + 300 });

type ExpiredListener = Parameters<BrowserProviderClient['subscribeSessionExpired']>[0];

function fakeProvider() {
  const expired = new Set<ExpiredListener>();
  const provider = {
    login: jest.fn(() => Promise.resolve()),
    handleCallback: jest.fn(() => Promise.resolve(false)),
    accessToken: jest.fn<Promise<string | undefined>, [{ readonly forceRefresh?: boolean }?]>(() =>
      Promise.resolve(token),
    ),
    subscribeSessionExpired: jest.fn((listener: ExpiredListener) => {
      expired.add(listener);
      return () => expired.delete(listener);
    }),
    logout: jest.fn<Promise<boolean>, []>(() => Promise.resolve(true)),
    clearSession: jest.fn(),
  };
  const expire = (reason: string) => {
    for (const listener of expired) listener({ type: 'session-expired', reason });
  };
  return { provider, expire };
}

describe('browserSignInClient', () => {
  it('builds the provider lazily and publishes the in-memory session', async () => {
    const { provider } = fakeProvider();
    const create = jest.fn(() => provider);
    const client = browserSignInClient(create);
    const sessions: unknown[] = [];
    client.subscribe((session) => sessions.push(session?.subject));

    expect(create).not.toHaveBeenCalled();
    await expect(client.initialize()).resolves.toMatchObject({ subject: 'user_1', accessToken: token });
    await expect(client.accessToken({ forceRefresh: true })).resolves.toBe(token);

    expect(create).toHaveBeenCalledTimes(1);
    expect(provider.handleCallback).toHaveBeenCalledTimes(1);
    expect(provider.accessToken).toHaveBeenLastCalledWith({ forceRefresh: true });
    expect(sessions).toEqual(['user_1']);
  });

  it('reports a redirect that has not completed as dismissed and a held session as success', async () => {
    const { provider } = fakeProvider();
    provider.accessToken.mockResolvedValue(undefined);
    await expect(browserSignInClient(() => provider).signIn()).resolves.toEqual({
      status: 'cancelled',
      reason: 'dismiss',
    });
    expect(provider.login).toHaveBeenCalledTimes(1);

    const signedIn = fakeProvider().provider;
    await expect(browserSignInClient(() => signedIn).signIn()).resolves.toEqual({
      status: 'success',
      subject: 'user_1',
    });
  });

  it('forwards expiry the app understands and drops the session for every reason', async () => {
    const { provider, expire } = fakeProvider();
    const client = browserSignInClient(() => provider);
    const sessions: unknown[] = [];
    const reasons: string[] = [];
    client.subscribe((session) => sessions.push(session?.subject));
    client.subscribeSessionExpired((event) => reasons.push(event.reason));
    await client.initialize();

    expire('refresh_rejected');
    expire('signed_out');
    expire('profile_erased');

    expect(reasons).toEqual(['refresh_rejected', 'profile_erased']);
    expect(sessions).toEqual(['user_1', undefined, undefined, undefined]);
  });

  it('maps sign-out and clearing onto the provider', async () => {
    const { provider } = fakeProvider();
    const client = browserSignInClient(() => provider);
    await expect(client.signOut()).resolves.toEqual({ providerLogout: 'completed' });

    provider.logout.mockResolvedValueOnce(false);
    await expect(client.signOut()).resolves.toEqual({ providerLogout: 'unavailable' });

    provider.logout.mockRejectedValueOnce(new Error('offline'));
    await expect(client.signOut()).resolves.toEqual({ providerLogout: 'failed' });

    await client.clearSession({ reason: 'profile_erased' });
    expect(provider.clearSession).toHaveBeenCalledWith({ reason: 'profile_erased' });
  });

  it('clears the published session when the provider no longer has a token', async () => {
    const { provider } = fakeProvider();
    const client = browserSignInClient(() => provider);
    const sessions: unknown[] = [];
    client.subscribe((session) => sessions.push(session?.subject));
    await client.initialize();

    provider.accessToken.mockResolvedValueOnce(undefined);
    await expect(client.accessToken()).resolves.toBeUndefined();

    expect(sessions).toEqual(['user_1', undefined]);
  });
});
