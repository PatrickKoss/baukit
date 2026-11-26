import { describe, expect, it } from 'vitest';

import { NativeOidcClient, type FetchPort, type SecureStoragePort } from './index.js';

const ISSUER = 'https://identity.example.test/tenant';
const SESSION_KEY = 'native-test.session';

class FlakyStorage implements SecureStoragePort {
  public readonly values = new Map<string, string>();
  public failNextSet = false;

  public get(key: string): Promise<string | null> {
    return Promise.resolve(this.values.get(key) ?? null);
  }

  public set(key: string, value: string): Promise<void> {
    if (this.failNextSet) {
      this.failNextSet = false;
      return Promise.reject(new Error('keychain locked'));
    }
    this.values.set(key, value);
    return Promise.resolve();
  }

  public delete(key: string): Promise<void> {
    this.values.delete(key);
    return Promise.resolve();
  }
}

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), { headers: { 'content-type': 'application/json' } });
}

function harness() {
  const storage = new FlakyStorage();
  storage.values.set(
    SESSION_KEY,
    JSON.stringify({
      subject: 'user',
      accessToken: 'expired-access',
      refreshToken: 'refresh-1',
      issuedAt: 0,
      expiresAt: 500,
    }),
  );
  const tokenCalls: string[] = [];
  const fetch: FetchPort = (url, init) => {
    if (url.endsWith('/.well-known/openid-configuration')) {
      return Promise.resolve(
        json({
          issuer: ISSUER,
          authorization_endpoint: 'https://login.example.test/authorize',
          token_endpoint: 'https://login.example.test/token',
          userinfo_endpoint: 'https://login.example.test/userinfo',
        }),
      );
    }
    tokenCalls.push((init?.body as URLSearchParams).get('refresh_token') ?? '');
    return Promise.resolve(
      json({ access_token: 'access-2', refresh_token: 'refresh-2', expires_in: 600 }),
    );
  };
  const client = new NativeOidcClient(
    {
      issuer: ISSUER,
      clientId: 'mobile',
      redirectUri: 'product://oauth',
      offlineAccess: true,
      storageKeyPrefix: 'native-test',
    },
    {
      fetch,
      storage,
      browser: {
        authorize: () => Promise.reject(new Error('unused')),
        endSession: () => Promise.resolve(true),
      },
      now: () => 1_000,
    },
  );
  return { client, storage, tokenCalls };
}

describe('NativeOidcClient refresh persistence', () => {
  it('keeps the rotated token in memory when storing it fails', async () => {
    const { client, storage, tokenCalls } = harness();
    await client.initialize();
    storage.failNextSet = true;
    await expect(client.accessToken()).rejects.toMatchObject({ code: 'storage_failed' });
    await expect(client.accessToken({ forceRefresh: true })).resolves.toBe('access-2');
    expect(tokenCalls).toEqual(['refresh-1', 'refresh-2']);
  });
});
