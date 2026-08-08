export interface OidcEndpoints {
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly revocationEndpoint: string;
  readonly endSessionEndpoint: string;
}

export function oidcEndpoints(issuer: string): OidcEndpoints {
  const normalized = issuer.replace(/\/$/u, '');
  return {
    authorizationEndpoint: `${normalized}/protocol/openid-connect/auth`,
    tokenEndpoint: `${normalized}/protocol/openid-connect/token`,
    revocationEndpoint: `${normalized}/protocol/openid-connect/revoke`,
    endSessionEndpoint: `${normalized}/protocol/openid-connect/logout`,
  };
}

export function logoutUrl(
  endpoint: string,
  clientId: string,
  redirectUri: string,
  idToken?: string,
): string {
  const url = new URL(endpoint);
  url.searchParams.set('client_id', clientId);
  url.searchParams.set('post_logout_redirect_uri', redirectUri);
  if (idToken !== undefined) {
    url.searchParams.set('id_token_hint', idToken);
  }
  return url.toString();
}
