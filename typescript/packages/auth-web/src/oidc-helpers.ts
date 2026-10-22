import type { OidcClientConfig, PkceAuthorizationRequest } from './index.js';

/** Removes trailing issuer slashes while rejecting query strings and fragments. */
export function normalizeIssuer(issuer: string): string {
  const trimmed = issuer.trim();
  if (trimmed.length === 0) {
    throw new TypeError('OIDC issuer must not be empty.');
  }
  const url = new URL(trimmed);
  if (url.search.length !== 0 || url.hash.length !== 0) {
    throw new TypeError('OIDC issuer must not include a query string or fragment.');
  }
  return url.toString().replace(/\/+$/u, '');
}

/** Builds an authorization request from the endpoint supplied by discovery. */
export function buildAuthorizationUrl(
  config: Pick<
    OidcClientConfig,
    'clientId' | 'redirectUri' | 'scopes' | 'offlineAccess' | 'audience' | 'resource'
  >,
  authorizationEndpoint: string,
  pkce: PkceAuthorizationRequest,
  prompt?: 'login',
): URL {
  const endpoint = new URL(authorizationEndpoint);
  endpoint.search = new URLSearchParams({
    client_id: requiredText(config.clientId, 'OIDC client ID'),
    redirect_uri: requiredUrl(config.redirectUri, 'OIDC redirect URI'),
    response_type: 'code',
    scope: normalizeScopes(config.scopes, config.offlineAccess).join(' '),
    state: pkce.state,
    code_challenge: pkce.challenge,
    code_challenge_method: 'S256',
  }).toString();
  if (config.audience !== undefined)
    endpoint.searchParams.set('audience', requiredText(config.audience, 'OIDC audience'));
  if (config.resource !== undefined)
    endpoint.searchParams.set('resource', requiredText(config.resource, 'OIDC resource'));
  if (prompt !== undefined) endpoint.searchParams.set('prompt', prompt);
  return endpoint;
}

export function normalizeScopes(
  scopes: readonly string[] | undefined,
  offlineAccess = false,
): string[] {
  const requested = scopes ?? ['openid', 'profile', 'email'];
  const normalized: string[] = [];
  for (const scope of ['openid', ...requested, ...(offlineAccess ? ['offline_access'] : [])]) {
    const trimmed = scope.trim();
    if (trimmed.length === 0 || /\s/u.test(trimmed)) {
      throw new TypeError('OIDC scopes must be non-empty and contain no whitespace.');
    }
    if (!normalized.includes(trimmed)) {
      normalized.push(trimmed);
    }
  }
  return normalized;
}

export function requiredText(value: string, label: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    throw new TypeError(`${label} must not be empty.`);
  }
  return trimmed;
}

export function requiredUrl(value: string, label: string): string {
  const trimmed = requiredText(value, label);
  try {
    return new URL(trimmed).toString();
  } catch {
    throw new TypeError(`${label} must be an absolute URL.`);
  }
}

export function randomUrlSafe(cryptoImplementation: Crypto, byteLength: number): string {
  return base64UrlEncode(cryptoImplementation.getRandomValues(new Uint8Array(byteLength)));
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '');
}
