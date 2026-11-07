import { isRecord } from '@baukit/type-guards';

import { OidcError } from './oidc-errors.js';
import { normalizeIssuer, requiredUrl } from './oidc-helpers.js';
import type { OidcProviderMetadata } from './index.js';

export interface StoredTokens {
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly issuedAt: number;
  readonly expiresAt: number;
}

export interface LoginTransaction {
  readonly state: string;
  readonly verifier: string;
}

export interface TokenResponse {
  readonly access_token: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
}

export interface RawProviderMetadata {
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly end_session_endpoint?: string;
}

export async function readProviderError(response: Response): Promise<string | undefined> {
  try {
    const value: unknown = await response.json();
    return isRecord(value) && typeof value['error'] === 'string' ? value['error'] : undefined;
  } catch {
    return undefined;
  }
}

export function readTransaction(raw: string | null): LoginTransaction | undefined {
  if (raw === null) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(raw);
    return isRecord(value) &&
      typeof value['state'] === 'string' &&
      value['state'].length > 0 &&
      typeof value['verifier'] === 'string' &&
      value['verifier'].length > 0
      ? { state: value['state'], verifier: value['verifier'] }
      : undefined;
  } catch {
    return undefined;
  }
}

export function isStoredTokens(value: unknown): value is StoredTokens {
  return (
    isRecord(value) &&
    typeof value['accessToken'] === 'string' &&
    value['accessToken'].length > 0 &&
    typeof value['issuedAt'] === 'number' &&
    Number.isFinite(value['issuedAt']) &&
    typeof value['expiresAt'] === 'number' &&
    Number.isFinite(value['expiresAt']) &&
    (value['refreshToken'] === undefined || typeof value['refreshToken'] === 'string') &&
    (value['idToken'] === undefined || typeof value['idToken'] === 'string')
  );
}

export function isTokenResponse(value: unknown): value is TokenResponse {
  return (
    isRecord(value) &&
    typeof value['access_token'] === 'string' &&
    value['access_token'].length > 0 &&
    typeof value['expires_in'] === 'number' &&
    Number.isFinite(value['expires_in']) &&
    value['expires_in'] > 0 &&
    (value['refresh_token'] === undefined || typeof value['refresh_token'] === 'string') &&
    (value['id_token'] === undefined || typeof value['id_token'] === 'string')
  );
}

export function isProviderMetadata(value: unknown): value is RawProviderMetadata {
  if (
    !isRecord(value) ||
    typeof value['issuer'] !== 'string' ||
    typeof value['authorization_endpoint'] !== 'string' ||
    typeof value['token_endpoint'] !== 'string' ||
    (value['end_session_endpoint'] !== undefined &&
      typeof value['end_session_endpoint'] !== 'string')
  ) {
    return false;
  }
  try {
    requiredUrl(value['authorization_endpoint'], 'OIDC authorization endpoint');
    requiredUrl(value['token_endpoint'], 'OIDC token endpoint');
    if (value['end_session_endpoint'] !== undefined) {
      requiredUrl(value['end_session_endpoint'], 'OIDC end-session endpoint');
    }
    return true;
  } catch {
    return false;
  }
}

export async function discoverProvider(
  issuer: string,
  fetchImplementation: typeof globalThis.fetch,
): Promise<OidcProviderMetadata> {
  let response: Response;
  try {
    response = await fetchImplementation(`${issuer}/.well-known/openid-configuration`);
  } catch {
    throw new OidcError('discovery_failed');
  }
  if (!response.ok) {
    throw new OidcError('discovery_failed', { status: response.status });
  }
  let value: unknown;
  try {
    value = await response.json();
  } catch {
    throw new OidcError('invalid_discovery_document');
  }
  if (!isProviderMetadata(value)) {
    throw new OidcError('invalid_discovery_document');
  }
  if (normalizeIssuer(value.issuer) !== issuer) {
    throw new OidcError('issuer_mismatch');
  }
  return {
    issuer,
    authorizationEndpoint: value.authorization_endpoint,
    tokenEndpoint: value.token_endpoint,
    ...(value.end_session_endpoint === undefined
      ? {}
      : { endSessionEndpoint: value.end_session_endpoint }),
  };
}
