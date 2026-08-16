import { describe, expect, it, vi } from 'vitest';

import { MockFetch } from '@baukit/api-runtime';
import { buildAuthorizationUrl } from '@baukit/auth-web';

import { createAuthenticatedApiRuntime } from './authenticated-api';
import { authClient } from './auth';

const unauthorized = {
  error: {
    code: 'unauthenticated',
    message: 'Authentication required',
    request_id: 'request-1',
    details: {},
  },
};

describe('OIDC authorization request', () => {
  it('requires authorization code with S256 PKCE', () => {
    const url = buildAuthorizationUrl(
      {
        clientId: 'product-web',
        redirectUri: 'https://app.example.test/',
        scopes: ['profile', 'email'],
        offlineAccess: true,
      },
      'https://login.example.test/authorize',
      { state: 'state-value', challenge: 'challenge-value' },
    );

    expect(authClient.hasSession()).toBe(false);
    expect(url.pathname).toBe('/authorize');
    expect(url.searchParams.get('response_type')).toBe('code');
    expect(url.searchParams.get('code_challenge_method')).toBe('S256');
    expect(url.searchParams.get('code_challenge')).toBe('challenge-value');
    expect(url.searchParams.get('state')).toBe('state-value');
    expect(url.searchParams.get('scope')).toBe('openid profile email offline_access');
  });

  it('refreshes credentials and replays a 401 once', async () => {
    const fetch = new MockFetch()
      .enqueueJson(unauthorized, { status: 401 })
      .enqueueJson({ id: 'user-1', subject: 'subject-1' });
    const accessToken = vi
      .fn()
      .mockResolvedValueOnce('expired-token')
      .mockResolvedValue('fresh-token');
    const runtime = createAuthenticatedApiRuntime({
      auth: { accessToken },
      baseUrl: 'https://api.example.test',
      environment: 'test',
      fetch: fetch.fetch,
    });

    await expect(runtime.fetch('/me')).resolves.toHaveProperty('status', 200);
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
      .fn()
      .mockResolvedValueOnce('expired-token')
      .mockResolvedValue('fresh-token');
    const runtime = createAuthenticatedApiRuntime({
      auth: { accessToken },
      baseUrl: 'https://api.example.test',
      environment: 'test',
      fetch: fetch.fetch,
    });

    await expect(runtime.fetch('/me')).rejects.toMatchObject({ status: 401 });
    expect(fetch.requests).toHaveLength(2);
    expect(accessToken).toHaveBeenCalledTimes(3);
  });
});
