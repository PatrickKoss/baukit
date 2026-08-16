import { describe, expect, it } from '@jest/globals';
import {
  NativeOidcClient,
  type AuthorizationRequest,
  type BrowserFlowPort,
  type SecureStoragePort,
} from '@baukit/auth-native';

import { signInFeedback } from './auth-feedback';

class MemoryStorage implements SecureStoragePort {
  private readonly values = new Map<string, string>();

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

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: () => Promise.resolve(body),
  } as Response;
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
