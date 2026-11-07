import { isRecord } from '@baukit/type-guards';

import type { FetchPort, NativeOidcConfig, OidcProviderMetadata, OidcSession } from './index.js';
import { OidcError } from './oidc-errors.js';

export interface NormalizedConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly audience?: string;
  readonly resource?: string;
  readonly scopes: readonly string[];
  readonly postLogoutRedirectUri: string;
  readonly refreshLeewayMs: number;
  readonly refreshOnRestore: boolean;
  readonly storageKeyPrefix: string;
}

export interface RawProviderMetadata {
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly userinfo_endpoint: string;
  readonly end_session_endpoint?: string;
}

export interface TokenResponse {
  readonly access_token: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
}

export interface ReceivedTokenResponse extends TokenResponse {
  readonly receivedAt: number;
}

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

export async function discoverProvider(
  issuer: string,
  fetchImplementation: FetchPort,
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
  return Object.freeze({
    issuer,
    authorizationEndpoint: value.authorization_endpoint,
    tokenEndpoint: value.token_endpoint,
    userInfoEndpoint: value.userinfo_endpoint,
    ...(value.end_session_endpoint === undefined
      ? {}
      : { endSessionEndpoint: value.end_session_endpoint }),
  });
}

export async function fetchSubject(
  endpoint: string,
  accessToken: string,
  fetchImplementation: FetchPort,
): Promise<string> {
  let response: Response;
  try {
    response = await fetchImplementation(endpoint, {
      headers: { authorization: `Bearer ${accessToken}` },
    });
  } catch {
    throw new OidcError('userinfo_failed');
  }
  if (!response.ok) {
    throw new OidcError('userinfo_failed', { status: response.status });
  }
  let value: unknown;
  try {
    value = await response.json();
  } catch {
    throw new OidcError('invalid_userinfo_response');
  }
  if (!isRecord(value) || typeof value['sub'] !== 'string' || value['sub'].length === 0) {
    throw new OidcError('invalid_userinfo_response');
  }
  return value['sub'];
}

export function normalizeConfig(config: NativeOidcConfig): NormalizedConfig {
  const issuer = normalizeIssuer(config.issuer);
  const clientId = requiredText(config.clientId, 'OIDC client ID');
  const redirectUri = requiredUrl(config.redirectUri, 'OIDC redirect URI');
  const refreshLeewaySeconds = config.refreshLeewaySeconds ?? 30;
  if (!Number.isFinite(refreshLeewaySeconds) || refreshLeewaySeconds < 0) {
    throw new RangeError('OIDC refresh leeway must be a non-negative number.');
  }
  const storageKeyPrefix =
    config.storageKeyPrefix ??
    `baukit.auth-native.${storageKeyPart(issuer)}.${storageKeyPart(clientId)}`;
  if (storageKeyPrefix.length === 0) {
    throw new TypeError('OIDC storage key prefix must not be empty.');
  }
  if (!/^[A-Za-z0-9._-]+$/u.test(storageKeyPrefix)) {
    throw new TypeError(
      "OIDC storage key prefix must contain only ASCII letters, digits, '.', '-' and '_'.",
    );
  }
  return {
    issuer,
    clientId,
    redirectUri,
    ...(config.audience === undefined
      ? {}
      : { audience: requiredText(config.audience, 'OIDC audience') }),
    ...(config.resource === undefined
      ? {}
      : { resource: requiredText(config.resource, 'OIDC resource') }),
    scopes: normalizeScopes(config.scopes, config.offlineAccess),
    postLogoutRedirectUri: requiredUrl(
      config.postLogoutRedirectUri ?? redirectUri,
      'OIDC post-logout redirect URI',
    ),
    refreshLeewayMs: refreshLeewaySeconds * 1000,
    refreshOnRestore: config.refreshOnRestore ?? false,
    storageKeyPrefix,
  };
}

export function storageKeyPart(value: string): string {
  return encodeURIComponent(value).replace(
    /[!'()*_.~%]/gu,
    (character) => `_${character.charCodeAt(0).toString(16)}`,
  );
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

export function refreshWindowMs(
  issuedAt: number,
  expiresAt: number,
  configuredLeewayMs: number,
): number {
  return Math.min(configuredLeewayMs, (expiresAt - issuedAt) / 2);
}

export function makeSession(
  tokens: ReceivedTokenResponse,
  subject: string,
  previous?: OidcSession,
): OidcSession {
  const refreshToken = tokens.refresh_token ?? previous?.refreshToken;
  const idToken = tokens.id_token ?? previous?.idToken;
  return Object.freeze({
    subject,
    accessToken: tokens.access_token,
    issuedAt: tokens.receivedAt,
    expiresAt: tokens.receivedAt + tokens.expires_in * 1000,
    ...(refreshToken === undefined ? {} : { refreshToken }),
    ...(idToken === undefined ? {} : { idToken }),
  });
}

export async function readProviderError(response: Response): Promise<string | undefined> {
  try {
    const value: unknown = await response.json();
    return isRecord(value) && typeof value['error'] === 'string' ? value['error'] : undefined;
  } catch {
    return undefined;
  }
}

export function isTerminalRefreshRejection(
  status: number,
  providerError: string | undefined,
): boolean {
  return (
    status === 400 ||
    status === 401 ||
    providerError === 'invalid_grant' ||
    providerError === 'invalid_token'
  );
}

export function isTerminalRefreshError(cause: unknown): boolean {
  return cause instanceof OidcError && cause.code === 'refresh_failed' && !cause.retryable;
}

export function transientRefreshError(cause: unknown): OidcError {
  if (cause instanceof OidcError && cause.code === 'refresh_failed' && cause.retryable) {
    return cause;
  }
  return new OidcError('refresh_failed', {
    retryable: true,
    ...(cause instanceof OidcError && cause.status !== undefined ? { status: cause.status } : {}),
  });
}

export function parseSession(raw: string | null): OidcSession | undefined {
  if (raw === null) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(raw);
    if (isSession(value)) {
      return Object.freeze({ ...value });
    }
  } catch {
    // Corrupt state is discarded by the caller without reflecting its content.
  }
  return undefined;
}

export function isSession(value: unknown): value is OidcSession {
  return (
    isRecord(value) &&
    typeof value['subject'] === 'string' &&
    value['subject'].length > 0 &&
    typeof value['accessToken'] === 'string' &&
    value['accessToken'].length > 0 &&
    typeof value['issuedAt'] === 'number' &&
    Number.isFinite(value['issuedAt']) &&
    typeof value['expiresAt'] === 'number' &&
    Number.isFinite(value['expiresAt']) &&
    (value['refreshToken'] === undefined ||
      (typeof value['refreshToken'] === 'string' && value['refreshToken'].length > 0)) &&
    (value['idToken'] === undefined ||
      (typeof value['idToken'] === 'string' && value['idToken'].length > 0))
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
    (value['refresh_token'] === undefined ||
      (typeof value['refresh_token'] === 'string' && value['refresh_token'].length > 0)) &&
    (value['id_token'] === undefined ||
      (typeof value['id_token'] === 'string' && value['id_token'].length > 0))
  );
}

export function isProviderMetadata(value: unknown): value is RawProviderMetadata {
  if (
    !isRecord(value) ||
    typeof value['issuer'] !== 'string' ||
    typeof value['authorization_endpoint'] !== 'string' ||
    typeof value['token_endpoint'] !== 'string' ||
    typeof value['userinfo_endpoint'] !== 'string' ||
    (value['end_session_endpoint'] !== undefined &&
      typeof value['end_session_endpoint'] !== 'string')
  ) {
    return false;
  }
  try {
    normalizeIssuer(value['issuer']);
    requiredUrl(value['authorization_endpoint'], 'OIDC authorization endpoint');
    requiredUrl(value['token_endpoint'], 'OIDC token endpoint');
    requiredUrl(value['userinfo_endpoint'], 'OIDC UserInfo endpoint');
    if (value['end_session_endpoint'] !== undefined) {
      requiredUrl(value['end_session_endpoint'], 'OIDC end-session endpoint');
    }
    return true;
  } catch {
    return false;
  }
}
