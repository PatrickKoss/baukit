import { describe, expect, it } from 'vitest';

import { buildAuthorizationUrl } from './auth';

describe('OIDC authorization request', () => {
  it('requires authorization code with S256 PKCE', () => {
    const url = buildAuthorizationUrl(
      {
        issuer: 'https://identity.example.test/realms/product',
        clientId: 'product-web',
        redirectUri: 'https://app.example.test/',
      },
      { state: 'state-value', challenge: 'challenge-value' },
    );

    expect(url.pathname).toBe('/realms/product/protocol/openid-connect/auth');
    expect(url.searchParams.get('response_type')).toBe('code');
    expect(url.searchParams.get('code_challenge_method')).toBe('S256');
    expect(url.searchParams.get('code_challenge')).toBe('challenge-value');
    expect(url.searchParams.get('state')).toBe('state-value');
  });
});
