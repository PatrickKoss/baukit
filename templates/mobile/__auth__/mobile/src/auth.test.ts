import { describe, expect, it } from '@jest/globals';

import { logoutUrl, oidcEndpoints } from './auth-urls';

describe('mobile OIDC endpoints', () => {
  it('uses the realm endpoints and a product-owned redirect URI', () => {
    const endpoints = oidcEndpoints('https://identity.example.test/realms/product/');
    expect(endpoints.authorizationEndpoint).toBe(
      'https://identity.example.test/realms/product/protocol/openid-connect/auth',
    );
    const url = new URL(
      logoutUrl(endpoints.endSessionEndpoint, 'product-mobile', 'product://oauth', 'id-token'),
    );
    expect(url.searchParams.get('client_id')).toBe('product-mobile');
    expect(url.searchParams.get('post_logout_redirect_uri')).toBe('product://oauth');
    expect(url.searchParams.get('id_token_hint')).toBe('id-token');
  });
});
