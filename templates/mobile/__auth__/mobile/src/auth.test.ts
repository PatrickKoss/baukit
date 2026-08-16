import { describe, expect, it } from '@jest/globals';
import {
  NativeOidcClient,
  type AuthorizationRequest,
  type BrowserFlowPort,
  type SecureStoragePort,
} from '@baukit/auth-native';

import { signInFeedback } from './auth-feedback';

class MemoryStorage implements SecureStoragePort {
  public readonly values = new Map<string, string>();

  public get(key: string): Promise<string | null> {
    return Promise.resolve(this.values.get(key) ?? null);
  }

  public set(key: string, value: string): Promise<void> {
    this.values.set(key, value);
    return Promise.resolve();
  }

  public delete(key: string): Promise<void> {
    this.values.delete(key);
    return Promise.resolve();
  }
}

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
  } as Response;
}

const issuer = 'https://identity.example.test/tenant';

function providerMetadata(): Response {
  return jsonResponse({
    issuer,
    authorization_endpoint: 'https://identity.example.test/authorize',
    token_endpoint: 'https://identity.example.test/token',
    userinfo_endpoint: 'https://identity.example.test/userinfo',
  });
}

function seedExpiredSession(storage: MemoryStorage): void {
  storage.values.set(
    'refresh-test:session',
    JSON.stringify({
      subject: 'subject-123',
      accessToken: 'expired-access',
      refreshToken: 'refresh-token',
      expiresAt: 900,
    }),
  );
}

function refreshClient(
  storage: MemoryStorage,
  fetch: (url: string) => Promise<Response>,
): NativeOidcClient {
  return new NativeOidcClient(
    {
      issuer,
      clientId: 'product-mobile',
      redirectUri: 'product://oauth',
      storageKeyPrefix: 'refresh-test',
    },
    {
      storage,
      browser: {
        authorize: () => Promise.resolve({ type: 'cancel' }),
        endSession: () => Promise.resolve(false),
      },
      now: () => 1_000,
      fetch,
    },
  );
}

describe('mobile OIDC integration', () => {
  it.each(['cancel', 'dismiss'] as const)(
    'keeps %s cancellation observable and non-error',
    (reason) => {
      const result = { status: 'cancelled', reason } as const;

      expect(signInFeedback(result)).toBe('Sign in cancelled. You can try again.');
      expect(result).toEqual({ status: 'cancelled', reason });
    },
  );

  it('shares one refresh across concurrent expired-token callers', async () => {
    const storage = new MemoryStorage();
    seedExpiredSession(storage);
    let tokenCalls = 0;
    let resolveRefresh: ((response: Response) => void) | undefined;
    let markStarted: (() => void) | undefined;
    const started = new Promise<void>((resolve) => {
      markStarted = resolve;
    });
    const client = refreshClient(storage, (url) => {
      if (url.endsWith('/.well-known/openid-configuration')) {
        return Promise.resolve(providerMetadata());
      }
      tokenCalls += 1;
      markStarted?.();
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    });

    const first = client.accessToken();
    const second = client.accessToken({ forceRefresh: true });
    await started;
    expect(tokenCalls).toBe(1);
    resolveRefresh?.(jsonResponse({ access_token: 'fresh-access', expires_in: 60 }));

    await expect(Promise.all([first, second])).resolves.toEqual(['fresh-access', 'fresh-access']);
    expect(tokenCalls).toBe(1);
  });

  it('preserves a session after transient refresh failure', async () => {
    const storage = new MemoryStorage();
    seedExpiredSession(storage);
    const client = refreshClient(storage, (url) =>
      Promise.resolve(
        url.endsWith('/.well-known/openid-configuration')
          ? providerMetadata()
          : jsonResponse({ error: 'temporarily_unavailable' }, 503),
      ),
    );

    await expect(client.accessToken()).rejects.toMatchObject({
      retryable: true,
      status: 503,
    });
    expect(client.session()?.subject).toBe('subject-123');
    expect(storage.values.has('refresh-test:session')).toBe(true);
  });

  it('makes terminal refresh rejection observable and expires the session', async () => {
    const storage = new MemoryStorage();
    seedExpiredSession(storage);
    const client = refreshClient(storage, (url) =>
      Promise.resolve(
        url.endsWith('/.well-known/openid-configuration')
          ? providerMetadata()
          : jsonResponse({ error: 'invalid_grant' }, 400),
      ),
    );
    const expired: unknown[] = [];
    client.subscribeSessionExpired((event) => expired.push(event));

    await expect(client.accessToken()).resolves.toBeUndefined();
    expect(client.session()).toBeUndefined();
    expect(expired).toEqual([{ type: 'session-expired', reason: 'refresh_rejected' }]);
  });

  it('forces credentials after provider logout fails', async () => {
    const storage = new MemoryStorage();
    const authorizationRequests: AuthorizationRequest[] = [];
    let authorizationCount = 0;
    const browser: BrowserFlowPort = {
      authorize(request) {
        authorizationRequests.push(request);
        authorizationCount += 1;
        return Promise.resolve(
          authorizationCount === 1
            ? {
                type: 'success',
                code: 'authorization-code',
                state: 'state',
                expectedState: 'state',
                codeVerifier: 'verifier',
              }
            : { type: 'cancel' },
        );
      },
      endSession() {
        return Promise.reject(new Error('provider unavailable'));
      },
    };
    const client = new NativeOidcClient(
      {
        issuer: 'https://identity.example.test/tenant',
        clientId: 'product-mobile',
        redirectUri: 'product://oauth',
        storageKeyPrefix: 'template-test',
      },
      {
        storage,
        browser,
        now: () => 1_000,
        fetch: (url) => {
          if (url.endsWith('/.well-known/openid-configuration')) {
            return Promise.resolve(
              jsonResponse({
                issuer: 'https://identity.example.test/tenant',
                authorization_endpoint: 'https://identity.example.test/authorize',
                token_endpoint: 'https://identity.example.test/token',
                userinfo_endpoint: 'https://identity.example.test/userinfo',
                end_session_endpoint: 'https://identity.example.test/logout',
              }),
            );
          }
          if (url.endsWith('/userinfo')) {
            return Promise.resolve(jsonResponse({ sub: 'subject-123' }));
          }
          return Promise.resolve(
            jsonResponse({
              access_token: 'access-token',
              refresh_token: 'refresh-token',
              id_token: 'id-token',
              expires_in: 300,
            }),
          );
        },
      },
    );

    await expect(client.signIn()).resolves.toEqual({
      status: 'success',
      subject: 'subject-123',
    });
    await expect(client.signOut()).resolves.toEqual({
      providerLogout: 'failed',
    });
    await expect(client.signIn()).resolves.toEqual({
      status: 'cancelled',
      reason: 'cancel',
    });
    expect(authorizationRequests.at(-1)?.prompt).toBe('login');
  });
});
