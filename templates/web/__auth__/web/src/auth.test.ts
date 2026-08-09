import { describe, expect, it } from 'vitest';

import { buildAuthorizationUrl } from '@baukit/auth-web';

import { authClient } from './auth';

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
});
