/// <reference types="node" />

import { createHash, randomBytes } from 'node:crypto';

import { NodeTokenCache, type CachedTokenProfile, type NodeTokenCacheOptions } from './cache.js';
import { AuthNodeError } from './errors.js';

const DEFAULT_SCOPES = ['openid', 'profile', 'offline_access'] as const;
const DEFAULT_DEVICE_INTERVAL_SECONDS = 5;
const SLOW_DOWN_INCREMENT_SECONDS = 5;
const DEFAULT_MAX_RESPONSE_BYTES = 64 * 1024;
const DEFAULT_REQUEST_TIMEOUT_MS = 15_000;
const DEFAULT_LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
const DEFAULT_REFRESH_LEEWAY_SECONDS = 60;
const DISPLAY_CLAIMS_MAX_BYTES = 16 * 1024;
const DEVICE_GRANT = 'urn:ietf:params:oauth:grant-type:device_code';

export interface EndpointPolicy {
  /** Additional issuer values accepted when discovery returns an alias. */
  readonly issuerAllowlist?: readonly string[];
  /** Additional origins allowed for discovered token and device endpoints. */
  readonly endpointOriginAllowlist?: readonly string[];
  /** Permits plain HTTP only for localhost and literal loopback addresses. */
  readonly allowLoopbackHttp?: boolean;
}

export interface DeviceFlowClientConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly scopes?: readonly string[];
  readonly audience?: string;
  readonly cache: Pick<NodeTokenCacheOptions, 'namespace'> &
    Partial<Omit<NodeTokenCacheOptions, 'namespace'>>;
  readonly profile?: string;
  readonly endpointPolicy?: EndpointPolicy;
  readonly maxResponseBytes?: number;
  readonly requestTimeoutMs?: number;
  readonly loginTimeoutMs?: number;
  readonly refreshLeewaySeconds?: number;
}

export type DeviceFlowStatus = 'waiting' | 'slow-down' | 'authorized';

export interface DeviceVerification {
  readonly verificationUri: string;
  readonly verificationUriComplete?: string;
  readonly userCode: string;
}

export interface DeviceFlowPresentation {
  readonly showVerification?: (verification: DeviceVerification) => void | Promise<void>;
  readonly showStatus?: (status: DeviceFlowStatus) => void | Promise<void>;
  readonly openBrowser?: (url: string) => void | Promise<void>;
}

export interface DeviceFlowEnvironment {
  readonly fetch?: typeof globalThis.fetch;
  readonly now?: () => number;
  readonly sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
  readonly signal?: AbortSignal;
  readonly environmentToken?: () => string | undefined | Promise<string | undefined>;
}

export interface AccessTokenOptions {
  readonly forceRefresh?: boolean;
  readonly signal?: AbortSignal;
}

export interface LoginOptions {
  readonly signal?: AbortSignal;
  readonly presentation?: DeviceFlowPresentation;
}

export interface OidcDeviceMetadata {
  readonly issuer: string;
  readonly deviceAuthorizationEndpoint: string;
  readonly tokenEndpoint: string;
}

export interface DisplayOnlyClaims {
  readonly subject?: string;
  readonly name?: string;
  readonly preferredUsername?: string;
  readonly email?: string;
  readonly expiresAt?: number;
}

interface NormalizedConfig {
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

interface DeviceAuthorization {
  readonly deviceCode: string;
  readonly userCode: string;
  readonly verificationUri: string;
  readonly verificationUriComplete?: string;
  readonly expiresIn: number;
  readonly interval: number;
  readonly codeVerifier: string;
}

interface TokenResponse {
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly tokenType: string;
  readonly expiresIn: number;
}

const refreshFlights = new Map<string, Promise<CachedTokenProfile>>();

/** Node OIDC device-flow client. It never writes to stdout, stderr, or a logger. */
export class DeviceFlowClient {
  readonly #config: NormalizedConfig;
  readonly #environment: Required<Pick<DeviceFlowEnvironment, 'fetch' | 'now' | 'sleep'>> &
    Pick<DeviceFlowEnvironment, 'signal' | 'environmentToken'>;
  readonly #cache: NodeTokenCache;
  #metadataPromise: Promise<OidcDeviceMetadata> | undefined;

  public constructor(config: DeviceFlowClientConfig, environment: DeviceFlowEnvironment = {}) {
    this.#config = normalizeConfig(config);
    this.#environment = {
      fetch: environment.fetch ?? globalThis.fetch,
      now: environment.now ?? Date.now,
      sleep: environment.sleep ?? abortableDelay,
      ...(environment.signal === undefined ? {} : { signal: environment.signal }),
      ...(environment.environmentToken === undefined
        ? {}
        : { environmentToken: environment.environmentToken }),
    };
    this.#cache = new NodeTokenCache({
      ...config.cache,
      now: config.cache.now ?? this.#environment.now,
      sleep: config.cache.sleep ?? this.#environment.sleep,
    });
  }

  public discover(signal?: AbortSignal): Promise<OidcDeviceMetadata> {
    const requestSignal = combineSignals(this.#environment.signal, signal);
    this.#metadataPromise ??= discoverDeviceProvider(
      this.#config.issuer,
      this.#config.endpointPolicy,
      this.#environment.fetch,
      {
        maxResponseBytes: this.#config.maxResponseBytes,
        requestTimeoutMs: this.#config.requestTimeoutMs,
        ...(requestSignal === undefined ? {} : { signal: requestSignal }),
      },
    ).catch((error: unknown) => {
      this.#metadataPromise = undefined;
      throw error;
    });
    return this.#metadataPromise;
  }

  public async login(options: LoginOptions = {}): Promise<CachedTokenProfile> {
    const parentSignal = combineSignals(this.#environment.signal, options.signal);
    const total = timeoutSignal(parentSignal, this.#config.loginTimeoutMs);
    try {
      const metadata = await this.discover(total.signal);
      const device = await this.#authorizeDevice(metadata, total.signal);
      const verification: DeviceVerification = {
        verificationUri: device.verificationUri,
        userCode: device.userCode,
        ...(device.verificationUriComplete === undefined
          ? {}
          : { verificationUriComplete: device.verificationUriComplete }),
      };
      await options.presentation?.showVerification?.(verification);
      const browserUri = device.verificationUriComplete ?? device.verificationUri;
      await options.presentation?.openBrowser?.(browserUri);
      const token = await this.#poll(metadata, device, total.signal, options.presentation);
      const profile = this.#toCachedProfile(token);
      await this.#cache.write(this.#config.profile, profile, total.signal);
      return profile;
    } catch (error) {
      throw mapAbort(error, parentSignal, total.didTimeout());
    } finally {
      total.dispose();
    }
  }

  public async accessToken(options: AccessTokenOptions = {}): Promise<string> {
    const environmentToken = (await this.#environment.environmentToken?.())?.trim();
    if (environmentToken) return environmentToken;
    const signal = combineSignals(this.#environment.signal, options.signal);
    throwIfAborted(signal);
    const cached = await this.#cache.read(this.#config.profile);
    if (cached === undefined || !this.#matchesConfig(cached)) {
      throw new AuthNodeError('no_cached_session');
    }
    if (options.forceRefresh !== true && this.#isFresh(cached)) return cached.accessToken;
    if (cached.refreshToken === undefined) throw new AuthNodeError('no_cached_session');

    const flightKey = `${this.#cache.path}\0${this.#config.profile}`;
    let flight = refreshFlights.get(flightKey);
    if (flight === undefined) {
      flight = this.#refreshUnderLock(cached, options.forceRefresh === true, signal);
      refreshFlights.set(flightKey, flight);
      const clearFlight = (): void => {
        if (refreshFlights.get(flightKey) === flight) refreshFlights.delete(flightKey);
      };
      void flight.then(clearFlight, clearFlight);
    }
    return (await flight).accessToken;
  }

  public logout(signal?: AbortSignal): Promise<boolean> {
    return this.#cache.remove(
      this.#config.profile,
      combineSignals(this.#environment.signal, signal),
    );
  }

  /** Returns unverified claims for display text only. Never use them for authorization or identity keys. */
  public async displayClaims(): Promise<DisplayOnlyClaims | undefined> {
    const cached = await this.#cache.read(this.#config.profile);
    if (cached === undefined) return undefined;
    return decodeDisplayOnlyClaims(cached.idToken ?? cached.accessToken);
  }

  async #authorizeDevice(
    metadata: OidcDeviceMetadata,
    signal: AbortSignal | undefined,
  ): Promise<DeviceAuthorization> {
    const codeVerifier = randomBytes(32).toString('base64url');
    const codeChallenge = createHash('sha256').update(codeVerifier).digest('base64url');
    const response = await postForm(
      metadata.deviceAuthorizationEndpoint,
      {
        client_id: this.#config.clientId,
        scope: this.#config.scopes.join(' '),
        code_challenge: codeChallenge,
        code_challenge_method: 'S256',
        ...(this.#config.audience === undefined ? {} : { audience: this.#config.audience }),
      },
      this.#environment.fetch,
      this.#requestOptions(signal),
      'invalid_device_response',
      'device_authorization_failed',
    );
    if (!response.response.ok) {
      throw new AuthNodeError('device_authorization_failed', { status: response.response.status });
    }
    const body = response.body;
    const verificationUri = requiredString(body, 'verification_uri', 'invalid_device_response');
    validateWebUrl(verificationUri, this.#config.endpointPolicy.allowLoopbackHttp);
    const verificationUriComplete = optionalString(body, 'verification_uri_complete');
    if (verificationUriComplete !== undefined) {
      validateWebUrl(verificationUriComplete, this.#config.endpointPolicy.allowLoopbackHttp);
    }
    return {
      deviceCode: requiredString(body, 'device_code', 'invalid_device_response'),
      userCode: requiredString(body, 'user_code', 'invalid_device_response'),
      verificationUri,
      ...(verificationUriComplete === undefined ? {} : { verificationUriComplete }),
      expiresIn: requiredPositiveInteger(body, 'expires_in', 'invalid_device_response'),
      interval:
        optionalPositiveInteger(body, 'interval', 'invalid_device_response') ??
        DEFAULT_DEVICE_INTERVAL_SECONDS,
      codeVerifier,
    };
  }

  async #poll(
    metadata: OidcDeviceMetadata,
    device: DeviceAuthorization,
    signal: AbortSignal | undefined,
    presentation: DeviceFlowPresentation | undefined,
  ): Promise<TokenResponse> {
    const deviceDeadline = this.#environment.now() + device.expiresIn * 1000;
    let intervalSeconds = device.interval;
    await presentation?.showStatus?.('waiting');
    while (this.#environment.now() < deviceDeadline) {
      await this.#environment.sleep(intervalSeconds * 1000, signal);
      throwIfAborted(signal);
      if (this.#environment.now() >= deviceDeadline) break;
      const result = await postForm(
        metadata.tokenEndpoint,
        {
          grant_type: DEVICE_GRANT,
          device_code: device.deviceCode,
          client_id: this.#config.clientId,
          code_verifier: device.codeVerifier,
        },
        this.#environment.fetch,
        this.#requestOptions(signal),
        'invalid_token_response',
        'device_authorization_failed',
      );
      if (result.response.ok) {
        await presentation?.showStatus?.('authorized');
        return parseTokenResponse(result.body);
      }
      const oauthError = optionalString(result.body, 'error');
      if (oauthError === 'authorization_pending') continue;
      if (oauthError === 'slow_down') {
        intervalSeconds += SLOW_DOWN_INCREMENT_SECONDS;
        await presentation?.showStatus?.('slow-down');
        continue;
      }
      if (oauthError === 'access_denied') throw new AuthNodeError('device_authorization_denied');
      if (oauthError === 'expired_token') throw new AuthNodeError('device_authorization_expired');
      throw new AuthNodeError('device_authorization_failed', { status: result.response.status });
    }
    throw new AuthNodeError('device_authorization_expired');
  }

  async #refreshUnderLock(
    observed: CachedTokenProfile,
    forceRefresh: boolean,
    signal: AbortSignal | undefined,
  ): Promise<CachedTokenProfile> {
    return this.#cache.withLock(async (transaction) => {
      const current = await transaction.read(this.#config.profile);
      if (current === undefined || !this.#matchesConfig(current)) {
        throw new AuthNodeError('no_cached_session');
      }
      if (
        current.accessToken !== observed.accessToken ||
        (!forceRefresh && this.#isFresh(current))
      ) {
        return current;
      }
      if (current.refreshToken === undefined) throw new AuthNodeError('no_cached_session');
      const metadata = await this.discover(signal);
      const result = await postForm(
        metadata.tokenEndpoint,
        {
          grant_type: 'refresh_token',
          refresh_token: current.refreshToken,
          client_id: this.#config.clientId,
          ...(this.#config.audience === undefined ? {} : { audience: this.#config.audience }),
        },
        this.#environment.fetch,
        this.#requestOptions(signal),
        'invalid_token_response',
        'refresh_failed',
      );
      if (!result.response.ok) {
        throw new AuthNodeError('refresh_failed', {
          retryable: result.response.status >= 500,
          status: result.response.status,
        });
      }
      const token = parseTokenResponse(result.body);
      const refreshed = this.#toCachedProfile(token, current.refreshToken);
      await transaction.write(this.#config.profile, refreshed);
      return refreshed;
    }, signal);
  }

  #toCachedProfile(token: TokenResponse, previousRefreshToken?: string): CachedTokenProfile {
    const refreshToken = token.refreshToken ?? previousRefreshToken;
    return {
      accessToken: token.accessToken,
      ...(refreshToken === undefined ? {} : { refreshToken }),
      ...(token.idToken === undefined ? {} : { idToken: token.idToken }),
      tokenType: token.tokenType,
      expiresAt: this.#environment.now() + token.expiresIn * 1000,
      issuer: this.#config.issuer,
      clientId: this.#config.clientId,
      scopes: this.#config.scopes,
      ...(this.#config.audience === undefined ? {} : { audience: this.#config.audience }),
    };
  }

  #isFresh(profile: CachedTokenProfile): boolean {
    return profile.expiresAt > this.#environment.now() + this.#config.refreshLeewayMs;
  }

  #matchesConfig(profile: CachedTokenProfile): boolean {
    return (
      profile.issuer === this.#config.issuer &&
      profile.clientId === this.#config.clientId &&
      profile.audience === this.#config.audience &&
      profile.scopes.length === this.#config.scopes.length &&
      profile.scopes.every((scope, index) => scope === this.#config.scopes[index])
    );
  }

  #requestOptions(signal?: AbortSignal): DeviceProviderRequestOptions {
    return {
      maxResponseBytes: this.#config.maxResponseBytes,
      requestTimeoutMs: this.#config.requestTimeoutMs,
      ...(signal === undefined ? {} : { signal }),
    };
  }
}

export interface DeviceProviderRequestOptions {
  readonly maxResponseBytes: number;
  readonly requestTimeoutMs: number;
  readonly signal?: AbortSignal;
}

export async function discoverDeviceProvider(
  configuredIssuer: string,
  policy: EndpointPolicy,
  fetchImplementation: typeof globalThis.fetch,
  options: DeviceProviderRequestOptions,
): Promise<OidcDeviceMetadata> {
  const normalizedPolicy = normalizePolicy(policy);
  const requestOptions: DeviceProviderRequestOptions = {
    maxResponseBytes: positiveInteger(options.maxResponseBytes, 'maximum response size'),
    requestTimeoutMs: positiveInteger(options.requestTimeoutMs, 'request timeout'),
    ...(options.signal === undefined ? {} : { signal: options.signal }),
  };
  const issuer = normalizeWebUrl(configuredIssuer, normalizedPolicy.allowLoopbackHttp, 'issuer');
  const discoveryUrl = `${issuer}/.well-known/openid-configuration`;
  let result: JsonResponse;
  try {
    result = await requestJson(
      discoveryUrl,
      { headers: { Accept: 'application/json' } },
      fetchImplementation,
      requestOptions,
      'invalid_discovery_document',
      'discovery_failed',
    );
  } catch (error) {
    if (error instanceof AuthNodeError) throw error;
    throw new AuthNodeError('discovery_failed');
  }
  if (!result.response.ok) {
    throw new AuthNodeError('discovery_failed', {
      retryable: result.response.status >= 500,
      status: result.response.status,
    });
  }
  const discoveredIssuer = normalizeWebUrl(
    requiredString(result.body, 'issuer', 'invalid_discovery_document'),
    normalizedPolicy.allowLoopbackHttp,
    'issuer',
  );
  const allowedIssuers = new Set([issuer, ...normalizedPolicy.issuerAllowlist]);
  if (!allowedIssuers.has(discoveredIssuer)) throw new AuthNodeError('issuer_mismatch');
  const tokenEndpoint = validateDiscoveredEndpoint(
    requiredString(result.body, 'token_endpoint', 'invalid_discovery_document'),
    discoveredIssuer,
    normalizedPolicy,
  );
  const deviceAuthorizationEndpoint = validateDiscoveredEndpoint(
    requiredString(result.body, 'device_authorization_endpoint', 'invalid_discovery_document'),
    discoveredIssuer,
    normalizedPolicy,
  );
  return { issuer: discoveredIssuer, tokenEndpoint, deviceAuthorizationEndpoint };
}

/** Decodes a small allowlist of unverified JWT claims for display text only. */
export function decodeDisplayOnlyClaims(token: string): DisplayOnlyClaims | undefined {
  try {
    const payload = token.split('.')[1];
    if (payload === undefined || Buffer.byteLength(payload) > DISPLAY_CLAIMS_MAX_BYTES)
      return undefined;
    const decoded = Buffer.from(payload, 'base64url').toString('utf8');
    if (Buffer.byteLength(decoded) > DISPLAY_CLAIMS_MAX_BYTES) return undefined;
    const value = JSON.parse(decoded) as unknown;
    if (!isRecord(value)) return undefined;
    const subject = optionalString(value, 'sub');
    const name = optionalString(value, 'name');
    const preferredUsername = optionalString(value, 'preferred_username');
    const email = optionalString(value, 'email');
    const expiresAt = optionalPositiveInteger(value, 'exp', 'invalid_token_response');
    return {
      ...(subject === undefined ? {} : { subject }),
      ...(name === undefined ? {} : { name }),
      ...(preferredUsername === undefined ? {} : { preferredUsername }),
      ...(email === undefined ? {} : { email }),
      ...(expiresAt === undefined ? {} : { expiresAt }),
    };
  } catch {
    return undefined;
  }
}

interface JsonResponse {
  readonly response: Response;
  readonly body: Record<string, unknown>;
}

async function postForm(
  endpoint: string,
  fields: Record<string, string>,
  fetchImplementation: typeof globalThis.fetch,
  options: DeviceProviderRequestOptions,
  invalidResponseCode:
    'invalid_device_response' | 'invalid_discovery_document' | 'invalid_token_response',
  requestFailureCode: 'device_authorization_failed' | 'discovery_failed' | 'refresh_failed',
): Promise<JsonResponse> {
  return requestJson(
    endpoint,
    {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/x-www-form-urlencoded',
      },
      body: new URLSearchParams(fields),
    },
    fetchImplementation,
    options,
    invalidResponseCode,
    requestFailureCode,
  );
}

async function requestJson(
  input: string,
  init: RequestInit,
  fetchImplementation: typeof globalThis.fetch,
  options: DeviceProviderRequestOptions,
  invalidResponseCode:
    'invalid_device_response' | 'invalid_discovery_document' | 'invalid_token_response',
  requestFailureCode: 'device_authorization_failed' | 'discovery_failed' | 'refresh_failed',
): Promise<JsonResponse> {
  const requestTimeout = timeoutSignal(options.signal, options.requestTimeoutMs);
  try {
    const response = await fetchImplementation(input, { ...init, signal: requestTimeout.signal });
    const declaredLength = Number(response.headers.get('content-length'));
    if (Number.isFinite(declaredLength) && declaredLength > options.maxResponseBytes) {
      throw new AuthNodeError(invalidResponseCode);
    }
    let text = '';
    if (response.body !== null) {
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let bytes = 0;
      for (;;) {
        const chunk = await reader.read();
        if (chunk.done) break;
        bytes += chunk.value.byteLength;
        if (bytes > options.maxResponseBytes) {
          await reader.cancel().catch(() => undefined);
          throw new AuthNodeError(invalidResponseCode);
        }
        text += decoder.decode(chunk.value, { stream: true });
      }
      text += decoder.decode();
    }
    let value: unknown;
    try {
      value = JSON.parse(text) as unknown;
    } catch {
      throw new AuthNodeError(invalidResponseCode);
    }
    if (!isRecord(value)) throw new AuthNodeError(invalidResponseCode);
    return { response, body: value };
  } catch (error) {
    const mapped = mapAbort(error, options.signal, requestTimeout.didTimeout());
    if (mapped instanceof AuthNodeError) throw mapped;
    throw new AuthNodeError(requestFailureCode, { retryable: true });
  } finally {
    requestTimeout.dispose();
  }
}

function parseTokenResponse(body: Record<string, unknown>): TokenResponse {
  const refreshToken = optionalString(body, 'refresh_token');
  const idToken = optionalString(body, 'id_token');
  return {
    accessToken: requiredString(body, 'access_token', 'invalid_token_response'),
    ...(refreshToken === undefined ? {} : { refreshToken }),
    ...(idToken === undefined ? {} : { idToken }),
    tokenType: optionalString(body, 'token_type') ?? 'Bearer',
    expiresIn: requiredPositiveInteger(body, 'expires_in', 'invalid_token_response'),
  };
}

function normalizeConfig(config: DeviceFlowClientConfig): NormalizedConfig {
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

function normalizePolicy(policy: EndpointPolicy): Required<EndpointPolicy> {
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

function validateDiscoveredEndpoint(
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

function normalizeWebUrl(value: string, allowLoopbackHttp: boolean, label: string): string {
  const url = parseUrl(requiredText(value, `OIDC ${label}`), 'config');
  if (url.search || url.hash || url.username || url.password) {
    throw new TypeError(`OIDC ${label} must not contain credentials, a query, or a fragment.`);
  }
  validateWebUrl(url.toString(), allowLoopbackHttp);
  return url.toString().replace(/\/+$/u, '');
}

function validateWebUrl(value: string, allowLoopbackHttp: boolean): void {
  const url = parseUrl(value, 'provider');
  if (url.protocol === 'https:') return;
  if (url.protocol === 'http:' && allowLoopbackHttp && isLoopback(url.hostname)) return;
  throw new AuthNodeError('endpoint_policy_violation');
}

function parseUrl(value: string, source: 'config' | 'provider'): URL {
  try {
    return new URL(value);
  } catch {
    if (source === 'config') throw new TypeError('OIDC URL is invalid.');
    throw new AuthNodeError('endpoint_policy_violation');
  }
}

function isLoopback(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/gu, '').toLowerCase();
  return host === 'localhost' || host === '::1' || /^127(?:\.\d{1,3}){3}$/u.test(host);
}

function normalizeScopes(scopes: readonly string[] | undefined): readonly string[] {
  const values = scopes ?? DEFAULT_SCOPES;
  const normalized = values.map((scope) => requiredText(scope, 'OIDC scope'));
  if (!normalized.includes('openid')) normalized.unshift('openid');
  return [...new Set(normalized)];
}

function requiredString(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_discovery_document' | 'invalid_token_response',
): string {
  const result = optionalString(value, key);
  if (result === undefined) throw new AuthNodeError(code);
  return result;
}

function optionalString(value: Record<string, unknown>, key: string): string | undefined {
  const result = value[key];
  return typeof result === 'string' && result.length > 0 ? result : undefined;
}

function requiredPositiveInteger(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_token_response',
): number {
  const result = optionalPositiveInteger(value, key, code);
  if (result === undefined) throw new AuthNodeError(code);
  return result;
}

function optionalPositiveInteger(
  value: Record<string, unknown>,
  key: string,
  code: 'invalid_device_response' | 'invalid_token_response',
): number | undefined {
  const result = value[key];
  if (result === undefined) return undefined;
  if (!Number.isSafeInteger(result) || (result as number) <= 0) throw new AuthNodeError(code);
  return result as number;
}

function requiredText(value: string, label: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0) throw new TypeError(`${label} must not be empty.`);
  return trimmed;
}

function positiveInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value <= 0)
    throw new RangeError(`${label} must be positive.`);
  return value;
}

function nonNegativeInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${label} must be a non-negative integer.`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted === true) throw new AuthNodeError('aborted');
}

function mapAbort(
  error: unknown,
  parentSignal: AbortSignal | undefined,
  timedOut: boolean,
): unknown {
  if (parentSignal?.aborted === true) return new AuthNodeError('aborted');
  if (timedOut) return new AuthNodeError('login_timeout', { retryable: true });
  return error;
}

function combineSignals(
  first: AbortSignal | undefined,
  second: AbortSignal | undefined,
): AbortSignal | undefined {
  if (first === undefined) return second;
  if (second === undefined) return first;
  return AbortSignal.any([first, second]);
}

function timeoutSignal(
  parent: AbortSignal | undefined,
  milliseconds: number,
): {
  readonly signal: AbortSignal;
  readonly didTimeout: () => boolean;
  readonly dispose: () => void;
} {
  const controller = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, milliseconds);
  const abortFromParent = (): void => {
    controller.abort();
  };
  parent?.addEventListener('abort', abortFromParent, { once: true });
  if (parent?.aborted === true) controller.abort();
  return {
    signal: controller.signal,
    didTimeout: () => timedOut,
    dispose: () => {
      clearTimeout(timeout);
      parent?.removeEventListener('abort', abortFromParent);
    },
  };
}

function abortableDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolveDelay, reject) => {
    if (signal?.aborted === true) {
      reject(new AuthNodeError('aborted'));
      return;
    }
    const onAbort = (): void => {
      clearTimeout(timeout);
      reject(new AuthNodeError('aborted'));
    };
    const timeout = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolveDelay();
    }, milliseconds);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}
