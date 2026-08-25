export type OidcErrorCode =
  | 'authorization_failed'
  | 'callback_state_mismatch'
  | 'discovery_failed'
  | 'invalid_discovery_document'
  | 'invalid_token_response'
  | 'issuer_mismatch'
  | 'refresh_failed'
  | 'token_exchange_failed';

const SAFE_ERROR_MESSAGES = {
  authorization_failed: 'OIDC authorization failed.',
  callback_state_mismatch: 'OIDC callback state did not match the login transaction.',
  discovery_failed: 'OIDC provider discovery failed.',
  invalid_discovery_document: 'OIDC provider returned invalid discovery metadata.',
  invalid_token_response: 'OIDC token endpoint returned an invalid response.',
  issuer_mismatch: 'OIDC provider issuer did not match the configured issuer.',
  refresh_failed: 'OIDC token refresh failed.',
  token_exchange_failed: 'OIDC token exchange failed.',
} as const satisfies Record<OidcErrorCode, string>;

/** An authentication failure whose message is safe to display or log. */
export class OidcError extends Error {
  public readonly code: OidcErrorCode;
  public readonly status: number | undefined;
  /** True only when retrying the failed operation may succeed without signing in again. */
  public readonly retryable: boolean;

  public constructor(
    code: OidcErrorCode,
    options: { readonly retryable?: boolean; readonly status?: number } = {},
  ) {
    super(SAFE_ERROR_MESSAGES[code]);
    this.name = 'OidcError';
    this.code = code;
    this.status = options.status;
    this.retryable = options.retryable ?? false;
  }
}

export interface OidcClientConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  /** `openid` is prepended when omitted. Defaults to `openid profile email`. */
  readonly scopes?: readonly string[];
  /** Adds `offline_access` to the requested scopes. Defaults to false. */
  readonly offlineAccess?: boolean;
  /** Defaults to `redirectUri`. */
  readonly postLogoutRedirectUri?: string;
  /** Defaults to 30 seconds. */
  readonly refreshLeewaySeconds?: number;
  /** Overrides the deterministic, issuer/client-specific storage prefix. */
  readonly storageKeyPrefix?: string;
}

export interface OidcProviderMetadata {
  readonly issuer: string;
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly endSessionEndpoint?: string;
}

export interface OidcClientEnvironment {
  readonly fetch: typeof globalThis.fetch;
  readonly crypto: Crypto;
  readonly localStorage: Storage;
  readonly sessionStorage: Storage;
  readonly currentUrl: () => string;
  readonly navigate: (url: string) => void;
  readonly replaceUrl: (url: string) => void;
  readonly now: () => number;
}

interface NormalizedConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly scopes: readonly string[];
  readonly postLogoutRedirectUri: string;
  readonly refreshLeewayMs: number;
  readonly storageKeyPrefix: string;
}

interface StoredTokens {
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly expiresAt: number;
}

interface LoginTransaction {
  readonly state: string;
  readonly verifier: string;
}

interface TokenResponse {
  readonly access_token: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
}

export type SessionExpiredReason = 'refresh_rejected' | 'refresh_unavailable';

export interface SessionExpiredEvent {
  readonly type: 'session-expired';
  readonly reason: SessionExpiredReason;
}

type SessionExpiredListener = (event: SessionExpiredEvent) => void;

interface RawProviderMetadata {
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly end_session_endpoint?: string;
}

export interface PkceAuthorizationRequest {
  readonly state: string;
  readonly challenge: string;
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

/** Builds an authorization request from the endpoint supplied by discovery. */
export function buildAuthorizationUrl(
  config: Pick<OidcClientConfig, 'clientId' | 'redirectUri' | 'scopes' | 'offlineAccess'>,
  authorizationEndpoint: string,
  pkce: PkceAuthorizationRequest,
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
  return endpoint;
}

/** Returns an allowlisted message without reflecting untrusted error content. */
export function safeAuthErrorMessage(cause: unknown): string {
  return cause instanceof OidcError ? SAFE_ERROR_MESSAGES[cause.code] : 'OIDC login failed.';
}

/** Browser OIDC authorization-code client. It has no React dependency. */
export class OidcClient {
  private readonly config: NormalizedConfig;
  private readonly environment: OidcClientEnvironment;
  private readonly tokenStorageKey: string;
  private readonly transactionStorageKey: string;
  private readonly sessionExpiredListeners = new Set<SessionExpiredListener>();
  private metadataPromise: Promise<OidcProviderMetadata> | undefined;
  private callbackPromise: Promise<boolean> | undefined;
  private refreshPromise: Promise<string | undefined> | undefined;
  private sessionRevision = 0;

  public constructor(
    config: OidcClientConfig,
    environment: OidcClientEnvironment = browserEnvironment(),
  ) {
    this.config = normalizeConfig(config);
    this.environment = environment;
    this.tokenStorageKey = `${this.config.storageKeyPrefix}:tokens`;
    this.transactionStorageKey = `${this.config.storageKeyPrefix}:transaction`;
  }

  public hasSession(): boolean {
    return this.readTokens() !== undefined;
  }

  public discover(): Promise<OidcProviderMetadata> {
    this.metadataPromise ??= discoverProvider(this.config.issuer, this.environment.fetch);
    return this.metadataPromise;
  }

  /** Observes terminal expiry separately from explicit logout or manual clearing. */
  public subscribeSessionExpired(listener: SessionExpiredListener): () => void {
    this.sessionExpiredListeners.add(listener);
    return () => {
      this.sessionExpiredListeners.delete(listener);
    };
  }

  public async login(): Promise<void> {
    const metadata = await this.discover();
    const state = randomUrlSafe(this.environment.crypto, 32);
    const verifier = randomUrlSafe(this.environment.crypto, 64);
    const challenge = base64UrlEncode(
      new Uint8Array(
        await this.environment.crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)),
      ),
    );
    this.environment.sessionStorage.setItem(
      this.transactionStorageKey,
      JSON.stringify({ state, verifier }),
    );
    const authorizationUrl = buildAuthorizationUrl(
      {
        clientId: this.config.clientId,
        redirectUri: this.config.redirectUri,
        scopes: this.config.scopes,
      },
      metadata.authorizationEndpoint,
      { state, challenge },
    );
    this.environment.navigate(authorizationUrl.toString());
  }

  /** Returns one shared promise so repeated effects cannot exchange a code twice. */
  public handleCallback(): Promise<boolean> {
    this.callbackPromise ??= this.handleCallbackOnce();
    return this.callbackPromise;
  }

  public async accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    const tokens = this.readTokens();
    if (tokens === undefined) {
      return undefined;
    }
    if (
      options.forceRefresh !== true &&
      tokens.expiresAt > this.environment.now() + this.config.refreshLeewayMs
    ) {
      return tokens.accessToken;
    }
    if (tokens.refreshToken === undefined) {
      this.expireSession('refresh_unavailable');
      return undefined;
    }
    if (this.refreshPromise !== undefined) {
      return this.refreshPromise;
    }
    const pending = this.refresh(tokens, tokens.refreshToken);
    this.refreshPromise = pending;
    try {
      return await pending;
    } finally {
      if (this.refreshPromise === pending) {
        this.refreshPromise = undefined;
      }
    }
  }

  /** Clears local tokens and redirects through the discovered logout endpoint when available. */
  public async logout(): Promise<boolean> {
    const idToken = this.readTokens()?.idToken;
    this.clearSession();
    const metadata = await this.discover();
    if (metadata.endSessionEndpoint === undefined) {
      return false;
    }
    const endpoint = new URL(metadata.endSessionEndpoint);
    endpoint.searchParams.set('client_id', this.config.clientId);
    endpoint.searchParams.set('post_logout_redirect_uri', this.config.postLogoutRedirectUri);
    if (idToken !== undefined) {
      endpoint.searchParams.set('id_token_hint', idToken);
    }
    this.environment.navigate(endpoint.toString());
    return true;
  }

  /** Clears local authentication state without contacting the provider. */
  public clearSession(): void {
    this.sessionRevision += 1;
    this.environment.localStorage.removeItem(this.tokenStorageKey);
    this.environment.sessionStorage.removeItem(this.transactionStorageKey);
  }

  private async handleCallbackOnce(): Promise<boolean> {
    const callback = new URL(this.environment.currentUrl());
    if (!hasCallbackParameters(callback)) {
      return false;
    }

    const code = callback.searchParams.get('code');
    const returnedState = callback.searchParams.get('state');
    const returnedIssuer = callback.searchParams.get('iss');
    const providerError = callback.searchParams.has('error');
    this.removeCallbackParameters(callback);

    const transaction = readTransaction(
      this.environment.sessionStorage.getItem(this.transactionStorageKey),
    );
    this.environment.sessionStorage.removeItem(this.transactionStorageKey);
    if (returnedState === null || transaction?.state !== returnedState) {
      throw new OidcError('callback_state_mismatch');
    }
    if (returnedIssuer !== null && normalizeIssuer(returnedIssuer) !== this.config.issuer) {
      throw new OidcError('issuer_mismatch');
    }
    if (providerError) {
      throw new OidcError('authorization_failed');
    }
    if (code === null) {
      throw new OidcError('callback_state_mismatch');
    }

    const sessionRevision = this.sessionRevision;
    const tokens = await this.exchange(
      new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: this.config.clientId,
        redirect_uri: this.config.redirectUri,
        code,
        code_verifier: transaction.verifier,
      }),
      undefined,
      'token_exchange_failed',
    );
    if (this.sessionRevision === sessionRevision) {
      this.writeTokens(tokens);
    }
    return true;
  }

  private removeCallbackParameters(callback: URL): void {
    for (const parameter of [
      'code',
      'state',
      'session_state',
      'iss',
      'error',
      'error_description',
      'error_uri',
    ]) {
      callback.searchParams.delete(parameter);
    }
    this.environment.replaceUrl(callback.toString());
  }

  private async refresh(tokens: StoredTokens, refreshToken: string): Promise<string | undefined> {
    const sessionRevision = this.sessionRevision;
    try {
      const refreshed = await this.exchange(
        new URLSearchParams({
          grant_type: 'refresh_token',
          client_id: this.config.clientId,
          refresh_token: refreshToken,
        }),
        tokens,
        'refresh_failed',
      );
      if (this.sessionRevision !== sessionRevision) {
        return this.readTokens()?.accessToken;
      }
      this.writeTokens(refreshed);
      return refreshed.accessToken;
    } catch (cause) {
      if (isTerminalRefreshError(cause)) {
        if (this.sessionRevision !== sessionRevision) {
          return this.readTokens()?.accessToken;
        }
        this.expireSession('refresh_rejected');
        return undefined;
      }
      throw transientRefreshError(cause);
    }
  }

  private async exchange(
    body: URLSearchParams,
    existingTokens: StoredTokens | undefined,
    failureCode: 'refresh_failed' | 'token_exchange_failed',
  ): Promise<StoredTokens> {
    const metadata = await this.discover();
    let response: Response;
    try {
      response = await this.environment.fetch(metadata.tokenEndpoint, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body,
      });
    } catch {
      throw new OidcError(failureCode, { retryable: failureCode === 'refresh_failed' });
    }
    const receivedAt = this.environment.now();
    if (!response.ok) {
      const providerError = await readProviderError(response);
      throw new OidcError(failureCode, {
        status: response.status,
        retryable:
          failureCode === 'refresh_failed' &&
          !isTerminalRefreshRejection(response.status, providerError),
      });
    }

    let payload: unknown;
    try {
      payload = await response.json();
    } catch {
      throw new OidcError('invalid_token_response');
    }
    if (!isTokenResponse(payload)) {
      throw new OidcError('invalid_token_response');
    }

    const refreshToken = payload.refresh_token ?? existingTokens?.refreshToken;
    const idToken = payload.id_token ?? existingTokens?.idToken;
    const tokens: StoredTokens = {
      accessToken: payload.access_token,
      expiresAt: receivedAt + payload.expires_in * 1000,
      ...(refreshToken === undefined ? {} : { refreshToken }),
      ...(idToken === undefined ? {} : { idToken }),
    };
    return tokens;
  }

  private writeTokens(tokens: StoredTokens): void {
    this.environment.localStorage.setItem(this.tokenStorageKey, JSON.stringify(tokens));
    this.sessionRevision += 1;
  }

  private expireSession(reason: SessionExpiredReason): void {
    this.clearSession();
    const event: SessionExpiredEvent = { type: 'session-expired', reason };
    for (const listener of this.sessionExpiredListeners) {
      try {
        listener(event);
      } catch {
        // Observers cannot change the terminal unauthenticated result.
      }
    }
  }

  private readTokens(): StoredTokens | undefined {
    const raw = this.environment.localStorage.getItem(this.tokenStorageKey);
    if (raw === null) {
      return undefined;
    }
    try {
      const value: unknown = JSON.parse(raw);
      if (isStoredTokens(value)) {
        return value;
      }
    } catch {
      // Invalid state is discarded below without reflecting its content.
    }
    this.environment.localStorage.removeItem(this.tokenStorageKey);
    this.sessionRevision += 1;
    return undefined;
  }
}

async function readProviderError(response: Response): Promise<string | undefined> {
  try {
    const value: unknown = await response.json();
    return isObject(value) && typeof value['error'] === 'string' ? value['error'] : undefined;
  } catch {
    return undefined;
  }
}

function isTerminalRefreshRejection(status: number, providerError: string | undefined): boolean {
  return (
    status === 400 ||
    status === 401 ||
    providerError === 'invalid_grant' ||
    providerError === 'invalid_token'
  );
}

function isTerminalRefreshError(cause: unknown): boolean {
  return cause instanceof OidcError && cause.code === 'refresh_failed' && !cause.retryable;
}

function transientRefreshError(cause: unknown): OidcError {
  if (cause instanceof OidcError && cause.code === 'refresh_failed' && cause.retryable) {
    return cause;
  }
  return new OidcError('refresh_failed', {
    retryable: true,
    ...(cause instanceof OidcError && cause.status !== undefined ? { status: cause.status } : {}),
  });
}

async function discoverProvider(
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

function normalizeConfig(config: OidcClientConfig): NormalizedConfig {
  const issuer = normalizeIssuer(config.issuer);
  const clientId = requiredText(config.clientId, 'OIDC client ID');
  const redirectUri = requiredUrl(config.redirectUri, 'OIDC redirect URI');
  const refreshLeewaySeconds = config.refreshLeewaySeconds ?? 30;
  if (!Number.isFinite(refreshLeewaySeconds) || refreshLeewaySeconds < 0) {
    throw new RangeError('OIDC refresh leeway must be a non-negative number.');
  }
  const storageKeyPrefix =
    config.storageKeyPrefix ?? `@baukit/auth-web:${encodeURIComponent(issuer)}:${clientId}`;
  if (storageKeyPrefix.length === 0) {
    throw new TypeError('OIDC storage key prefix must not be empty.');
  }
  return {
    issuer,
    clientId,
    redirectUri,
    scopes: normalizeScopes(config.scopes, config.offlineAccess),
    postLogoutRedirectUri: requiredUrl(
      config.postLogoutRedirectUri ?? redirectUri,
      'OIDC post-logout redirect URI',
    ),
    refreshLeewayMs: refreshLeewaySeconds * 1000,
    storageKeyPrefix,
  };
}

function normalizeScopes(scopes: readonly string[] | undefined, offlineAccess = false): string[] {
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

function requiredText(value: string, label: string): string {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    throw new TypeError(`${label} must not be empty.`);
  }
  return trimmed;
}

function requiredUrl(value: string, label: string): string {
  const trimmed = requiredText(value, label);
  try {
    return new URL(trimmed).toString();
  } catch {
    throw new TypeError(`${label} must be an absolute URL.`);
  }
}

function hasCallbackParameters(url: URL): boolean {
  return ['code', 'state', 'error', 'iss', 'session_state'].some((name) =>
    url.searchParams.has(name),
  );
}

function randomUrlSafe(cryptoImplementation: Crypto, byteLength: number): string {
  return base64UrlEncode(cryptoImplementation.getRandomValues(new Uint8Array(byteLength)));
}

function base64UrlEncode(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '');
}

function readTransaction(raw: string | null): LoginTransaction | undefined {
  if (raw === null) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(raw);
    return isObject(value) &&
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

function isStoredTokens(value: unknown): value is StoredTokens {
  return (
    isObject(value) &&
    typeof value['accessToken'] === 'string' &&
    value['accessToken'].length > 0 &&
    typeof value['expiresAt'] === 'number' &&
    Number.isFinite(value['expiresAt']) &&
    (value['refreshToken'] === undefined || typeof value['refreshToken'] === 'string') &&
    (value['idToken'] === undefined || typeof value['idToken'] === 'string')
  );
}

function isTokenResponse(value: unknown): value is TokenResponse {
  return (
    isObject(value) &&
    typeof value['access_token'] === 'string' &&
    value['access_token'].length > 0 &&
    typeof value['expires_in'] === 'number' &&
    Number.isFinite(value['expires_in']) &&
    value['expires_in'] > 0 &&
    (value['refresh_token'] === undefined || typeof value['refresh_token'] === 'string') &&
    (value['id_token'] === undefined || typeof value['id_token'] === 'string')
  );
}

function isProviderMetadata(value: unknown): value is RawProviderMetadata {
  if (
    !isObject(value) ||
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

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function browserEnvironment(): OidcClientEnvironment {
  if (typeof window === 'undefined') {
    throw new Error('OidcClient requires a browser environment or an explicit environment.');
  }
  return {
    fetch: globalThis.fetch.bind(globalThis),
    crypto: globalThis.crypto,
    localStorage: window.localStorage,
    sessionStorage: window.sessionStorage,
    currentUrl: () => window.location.href,
    navigate: (url) => {
      window.location.assign(url);
    },
    replaceUrl: (url) => {
      window.history.replaceState({}, '', url);
    },
    now: () => Date.now(),
  };
}
