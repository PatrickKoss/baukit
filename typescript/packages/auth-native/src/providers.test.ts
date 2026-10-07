import { describe, expect, it, vi } from 'vitest';
import { ClerkNativeClient } from './clerk.js';
import { createWorkOsNativeClient, tokenTiming } from './workos.js';
import type { NativeOidcEnvironment } from './index.js';

function token(subject = 'user_123', expires = 300, sid = 'session_123'): string {
  return `header.${btoa(JSON.stringify({ sub: subject, exp: expires, sid }))}.signature`;
}

function requestPayload(body: BodyInit | null | undefined): unknown {
  if (typeof body !== 'string') throw new Error('Expected JSON request body');
  return JSON.parse(body);
}

it('rejects missing and malformed scheduling claims without exposing token text', () => {
  for (const value of [
    'secret',
    'header.invalid.signature',
    `header.${btoa('{"sub":false,"exp":30}')}.signature`,
  ])
    expect(() => tokenTiming(value)).toThrow('OIDC token endpoint returned an invalid response.');
});

describe('Clerk native contract', () => {
  it('waits for the SDK bridge, refreshes, emits expiry and signs out', async () => {
    const client = new ClerkNativeClient();
    const initialize = client.initialize();
    const getToken = vi
      .fn<(skipCache: boolean) => Promise<string | null>>()
      .mockResolvedValue(token());
    const signOut = vi.fn<() => Promise<void>>().mockResolvedValue();
    client.bind({
      subject: () => 'user_123',
      getToken,
      signIn: () => Promise.resolve(true),
      signOut,
    });
    expect((await initialize)?.subject).toBe('user_123');
    const changes = vi.fn();
    const expired = vi.fn();
    client.subscribe(changes);
    client.subscribeSessionExpired(expired);
    expect(await client.signIn()).toEqual({ status: 'success', subject: 'user_123' });
    expect(getToken).toHaveBeenLastCalledWith(true);
    getToken.mockResolvedValue(null);
    expect(await client.accessToken()).toBeUndefined();
    expect(changes).toHaveBeenCalledWith(undefined);
    expect(expired).toHaveBeenCalledWith({ type: 'session-expired', reason: 'refresh_rejected' });
    expect(await client.signOut()).toEqual({ providerLogout: 'completed' });
    expect(signOut).toHaveBeenCalledTimes(1);
  });
  it('handles cancellation, subject mismatch and logout failure', async () => {
    const client = new ClerkNativeClient();
    const getToken = vi.fn<() => Promise<string | null>>().mockResolvedValue(token('another_user'));
    client.bind({
      subject: () => 'user_123',
      getToken,
      signIn: () => Promise.resolve(false),
      signOut: () => Promise.reject(new Error('network')),
    });
    expect(await client.signIn()).toEqual({ status: 'cancelled', reason: 'cancel' });
    await expect(client.initialize()).rejects.toMatchObject({ code: 'invalid_token_response' });
    expect(await client.signOut()).toEqual({ providerLogout: 'failed' });
    expect(client.session()).toBeUndefined();
  });
});

it('uses WorkOS public-client PKCE, JSON exchange, refresh rotation and session logout', async () => {
  const values = new Map<string, string>();
  let now = 1000;
  const authorize = vi.fn<NativeOidcEnvironment['browser']['authorize']>().mockResolvedValue({
    type: 'success',
    code: 'code_123',
    state: 'state',
    expectedState: 'state',
    codeVerifier: 'verifier',
  });
  const endSession = vi
    .fn<NativeOidcEnvironment['browser']['endSession']>()
    .mockImplementation(() => {
      expect([...values.keys()].some((key) => key.endsWith('.session'))).toBe(false);
      return Promise.resolve(true);
    });
  const fetch = vi
    .fn<NativeOidcEnvironment['fetch']>()
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ access_token: token(), refresh_token: 'refresh_1' })),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({ access_token: token('user_123', 600), refresh_token: 'refresh_2' }),
      ),
    );
  const environment: NativeOidcEnvironment = {
    fetch,
    now: () => now,
    browser: { authorize, endSession },
    storage: {
      get: (key) => Promise.resolve(values.get(key) ?? null),
      set: (key, value) => {
        values.set(key, value);
        return Promise.resolve();
      },
      delete: (key) => {
        values.delete(key);
        return Promise.resolve();
      },
    },
  };
  const client = createWorkOsNativeClient(
    {
      issuer: 'https://api.workos.com/',
      clientId: 'client_app',
      redirectUri: 'app://oauth',
      offlineAccess: true,
    },
    environment,
  );
  expect(await client.signIn()).toEqual({ status: 'success', subject: 'user_123' });
  expect(authorize).toHaveBeenCalledWith(
    expect.objectContaining({
      authorizationEndpoint: 'https://api.workos.com/user_management/authorize?provider=authkit',
      clientId: 'client_app',
    }),
  );
  expect(fetch.mock.calls[0]?.[0]).toBe('https://api.workos.com/user_management/authenticate');
  expect(requestPayload(fetch.mock.calls[0]?.[1]?.body)).toEqual({
    grant_type: 'authorization_code',
    client_id: 'client_app',
    code: 'code_123',
    redirect_uri: 'app://oauth',
    code_verifier: 'verifier',
  });
  now = 290000;
  expect(await client.accessToken()).toBe(token('user_123', 600));
  expect(client.session()?.refreshToken).toBe('refresh_2');
  expect(requestPayload(fetch.mock.calls[1]?.[1]?.body)).toEqual({
    grant_type: 'refresh_token',
    client_id: 'client_app',
    refresh_token: 'refresh_1',
  });
  expect(await client.signOut()).toEqual({ providerLogout: 'completed' });
  expect(endSession).toHaveBeenCalledWith({
    url: 'https://api.workos.com/user_management/sessions/logout?session_id=session_123&return_to=app%3A%2F%2Foauth',
    redirectUri: 'app://oauth',
  });
});
