/// <reference types="node" />

import { createHash, randomBytes } from 'node:crypto';

import { isRecord } from '@baukit/type-guards';

import { NodeTokenCache, type CachedTokenProfile, type NodeTokenCacheOptions } from './cache.js';
import {
  normalizeConfig,
  normalizePolicy,
  normalizeWebUrl,
  optionalPositiveInteger,
  optionalString,
  positiveInteger,
  requiredPositiveInteger,
  requiredString,
  validateDiscoveredEndpoint,
  validateWebUrl,
  type NormalizedConfig,
} from './device-flow-config.js';
import {
  abortableDelay,
  combineSignals,
  mapAbort,
  postForm,
  requestJson,
  throwIfAborted,
  timeoutSignal,
  type JsonResponse,
} from './device-flow-http.js';
import { AuthNodeError } from './errors.js';

const DEFAULT_DEVICE_INTERVAL_SECONDS = 5;
const SLOW_DOWN_INCREMENT_SECONDS = 5;
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
  /** Defaults to 60 seconds. */
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

function refreshWindowMs(issuedAt: number, expiresAt: number, configuredLeewayMs: number): number {
  return Math.min(configuredLeewayMs, (expiresAt - issuedAt) / 2);
}

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
    if (environmentToken !== undefined && environmentToken !== '') return environmentToken;
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
      {
        fetchImplementation: this.#environment.fetch,
        options: this.#requestOptions(signal),
        invalidResponseCode: 'invalid_device_response',
        requestFailureCode: 'device_authorization_failed',
      },
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
        {
          fetchImplementation: this.#environment.fetch,
          options: this.#requestOptions(signal),
          invalidResponseCode: 'invalid_token_response',
          requestFailureCode: 'device_authorization_failed',
        },
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
        {
          fetchImplementation: this.#environment.fetch,
          options: this.#requestOptions(signal),
          invalidResponseCode: 'invalid_token_response',
          requestFailureCode: 'refresh_failed',
        },
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
    const issuedAt = this.#environment.now();
    return {
      accessToken: token.accessToken,
      ...(refreshToken === undefined ? {} : { refreshToken }),
      ...(token.idToken === undefined ? {} : { idToken: token.idToken }),
      tokenType: token.tokenType,
      issuedAt,
      expiresAt: issuedAt + token.expiresIn * 1000,
      issuer: this.#config.issuer,
      clientId: this.#config.clientId,
      scopes: this.#config.scopes,
      ...(this.#config.audience === undefined ? {} : { audience: this.#config.audience }),
    };
  }

  #isFresh(profile: CachedTokenProfile): boolean {
    const leewayMs = refreshWindowMs(
      profile.issuedAt,
      profile.expiresAt,
      this.#config.refreshLeewayMs,
    );
    return profile.expiresAt > this.#environment.now() + leewayMs;
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
      {
        fetchImplementation: fetchImplementation,
        options: requestOptions,
        invalidResponseCode: 'invalid_discovery_document',
        requestFailureCode: 'discovery_failed',
      },
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
