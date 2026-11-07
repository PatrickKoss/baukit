import { isSafeInteger } from '@baukit/type-guards';

import type { DeviceFlowClientConfig, EndpointPolicy } from './device-flow.js';
import { isAllowedWebScheme } from './api-origin.js';
import { AuthNodeError } from './errors.js';

const DEFAULT_SCOPES = ['openid', 'profile', 'offline_access'] as const;

export const DEFAULT_MAX_RESPONSE_BYTES = 64 * 1024;
export const DEFAULT_REQUEST_TIMEOUT_MS = 15_000;
export const DEFAULT_LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
export const DEFAULT_REFRESH_LEEWAY_SECONDS = 60;
export interface NormalizedConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly scopes: readonly string[];
  readonly audience?: string;
  readonly profile: string;
  readonly endpointPolicy: Required<EndpointPolicy>;
  readonly maxResponseBytes: number;
  readonly requestTimeoutMs: number;
  readonly loginTimeoutMs: number;
  readonly refreshLeewayMs: number;
}

export function normalizeConfig(config: DeviceFlowClientConfig): NormalizedConfig {
  const policy = normalizePolicy(config.endpointPolicy ?? {});
  const issuer = normalizeWebUrl(config.issuer, policy.allowLoopbackHttp, 'issuer');
  const clientId = requiredText(config.clientId, 'OIDC client ID');
  const scopes = normalizeScopes(config.scopes);
  return {
    issuer,
    clientId,
    scopes,
    ...(config.audience === undefined
      ? {}
      : { audience: requiredText(config.audience, 'audience') }),
    profile: config.profile ?? 'default',
    endpointPolicy: policy,
    maxResponseBytes: positiveInteger(
      config.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES,
      'maximum response size',
    ),
    requestTimeoutMs: positiveInteger(
      config.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS,
      'request timeout',
    ),
    loginTimeoutMs: positiveInteger(
      config.loginTimeoutMs ?? DEFAULT_LOGIN_TIMEOUT_MS,
      'login timeout',
    ),
    refreshLeewayMs:
      nonNegativeInteger(
        config.refreshLeewaySeconds ?? DEFAULT_REFRESH_LEEWAY_SECONDS,
        'refresh leeway',
      ) * 1000,
  };
}

export function normalizePolicy(policy: EndpointPolicy): Required<EndpointPolicy> {
  const allowLoopbackHttp = policy.allowLoopbackHttp ?? false;
  return {
    allowLoopbackHttp,
    issuerAllowlist: (policy.issuerAllowlist ?? []).map((issuer) =>
      normalizeWebUrl(issuer, allowLoopbackHttp, 'issuer'),
    ),
    endpointOriginAllowlist: (policy.endpointOriginAllowlist ?? []).map((origin) => {
      const url = parseUrl(origin, 'config');
      if (url.origin !== url.toString().replace(/\/$/u, '') || url.username || url.password) {
        throw new TypeError('endpoint origin allowlist entries must contain only an origin.');
      }
      validateWebUrl(url.toString(), allowLoopbackHttp);
      return url.origin;
    }),
  };
}

export function validateDiscoveredEndpoint(
  value: string,
  issuer: string,
  policy: Required<EndpointPolicy>,
): string {
  const endpoint = parseUrl(value, 'provider');
  validateWebUrl(endpoint.toString(), policy.allowLoopbackHttp);
  if (endpoint.username || endpoint.password || endpoint.hash) {
    throw new AuthNodeError('endpoint_policy_violation');
  }
  const issuerUrl = parseUrl(issuer, 'provider');
  const originAllowed =
    endpoint.origin === issuerUrl.origin ||
    policy.endpointOriginAllowlist.includes(endpoint.origin);
  if (!originAllowed) throw new AuthNodeError('endpoint_policy_violation');
  if (endpoint.origin === issuerUrl.origin) {
    const issuerPath = issuerUrl.pathname.replace(/\/$/u, '');
    const relatedPath =
      issuerPath.length === 0 ||
      endpoint.pathname === issuerPath ||
      endpoint.pathname.startsWith(`${issuerPath}/`);
    if (!relatedPath) throw new AuthNodeError('endpoint_policy_violation');
  }
  return endpoint.toString();
}

export function normalizeWebUrl(value: string, allowLoopbackHttp: boolean, label: string): string {
  const url = parseUrl(requiredText(value, `OIDC ${label}`), 'config');
  if (url.search || url.hash || url.username || url.password) {
    throw new TypeError(`OIDC ${label} must not contain credentials, a query, or a fragment.`);
  }
  validateWebUrl(url.toString(), allowLoopbackHttp);
  return url.toString().replace(/\/+$/u, '');
}

export function validateWebUrl(value: string, allowLoopbackHttp: boolean): void {
  if (!isAllowedWebScheme(parseUrl(value, 'provider'), allowLoopbackHttp)) {
    throw new AuthNodeError('endpoint_policy_violation');
  }
}

export function parseUrl(value: string, source: 'config' | 'provider'): URL {
  try {
    return new URL(value);
  } catch {
    if (source === 'config') throw new TypeError('OIDC URL is invalid.');
    throw new AuthNodeError('endpoint_policy_violation');
  }
}

export function normalizeScopes(scopes: readonly string[] | undefined): readonly string[] {
  const values = scopes ?? DEFAULT_SCOPES;
  const normalized = values.map((scope) => requiredText(scope, 'OIDC scope'));
  if (!normalized.includes('openid')) normalized.unshift('openid');
  return [...new Set(normalized)];
}

export function requiredString(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_discovery_document' | 'invalid_token_response',
): string {
  const result = optionalString(value, key);
  if (result === undefined) throw new AuthNodeError(code);
  return result;
}

export function optionalString(value: Record<string, unknown>, key: string): string | undefined {
  const result = value[key];
  return typeof result === 'string' && result.length > 0 ? result : undefined;
}

export function requiredPositiveInteger(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_token_response',
): number {
  const result = optionalPositiveInteger(value, key, code);
  if (result === undefined) throw new AuthNodeError(code);
  return result;
}

export function optionalPositiveInteger(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_token_response',
): number | undefined {
  const result = value[key];
  if (result === undefined) return undefined;
  if (!isSafeInteger(result) || result <= 0) throw new AuthNodeError(code);
  return result;
}

export function requiredText(value: string, label: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0) throw new TypeError(`${label} must not be empty.`);
  return trimmed;
}

export function positiveInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value <= 0)
    throw new RangeError(`${label} must be positive.`);
  return value;
}

export function nonNegativeInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${label} must be a non-negative integer.`);
  }
  return value;
}
