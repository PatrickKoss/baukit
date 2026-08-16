export type OidcErrorCode =
  | 'authorization_failed'
  | 'callback_state_mismatch'
  | 'discovery_failed'
  | 'invalid_discovery_document'
  | 'invalid_token_response'
  | 'invalid_userinfo_response'
  | 'issuer_mismatch'
  | 'pkce_verifier_missing'
  | 'refresh_failed'
  | 'storage_failed'
  | 'token_exchange_failed'
  | 'userinfo_failed';

const SAFE_ERROR_MESSAGES = {
  authorization_failed: 'OIDC authorization failed.',
  callback_state_mismatch: 'OIDC callback state did not match the login transaction.',
  discovery_failed: 'OIDC provider discovery failed.',
  invalid_discovery_document: 'OIDC provider returned invalid discovery metadata.',
  invalid_token_response: 'OIDC token endpoint returned an invalid response.',
  invalid_userinfo_response: 'OIDC UserInfo endpoint returned an invalid response.',
  issuer_mismatch: 'OIDC provider issuer did not match the configured issuer.',
  pkce_verifier_missing: 'OIDC login did not produce a PKCE verifier.',
  refresh_failed: 'OIDC token refresh failed.',
  storage_failed: 'Secure authentication storage failed.',
  token_exchange_failed: 'OIDC token exchange failed.',
  userinfo_failed: 'OIDC UserInfo request failed.',
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

export interface NativeOidcConfig {
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
  /** Overrides the deterministic issuer/client-specific storage prefix. */
  readonly storageKeyPrefix?: string;
}

export interface OidcProviderMetadata {
  readonly issuer: string;
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly userInfoEndpoint: string;
  readonly endSessionEndpoint?: string;
}

export interface OidcSession {
  /** Immutable OIDC subject obtained from the authenticated UserInfo response. */
  readonly subject: string;
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly expiresAt: number;
}

export type SignInResult =
  | { readonly status: 'success'; readonly subject: string }
  | { readonly status: 'cancelled'; readonly reason: 'cancel' | 'dismiss' };

export type SignOutResult =
  | { readonly providerLogout: 'completed' }
  | { readonly providerLogout: 'unavailable' }
  | { readonly providerLogout: 'failed' };

export type SessionExpiredReason = 'refresh_rejected' | 'refresh_unavailable';

export interface SessionExpiredEvent {
  readonly type: 'session-expired';
  readonly reason: SessionExpiredReason;
}

export interface SecureStoragePort {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export interface AuthorizationRequest {
  readonly authorizationEndpoint: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly scopes: readonly string[];
  readonly prompt?: 'login';
}

export type AuthorizationResult =
  | { readonly type: 'cancel' }
  | { readonly type: 'dismiss' }
  | { readonly type: 'error' }
  | {
      readonly type: 'success';
      readonly code: string;
      readonly state: string;
      readonly expectedState: string;
      readonly codeVerifier: string;
    };

export interface EndSessionRequest {
  readonly url: string;
  readonly redirectUri: string;
}

/** AuthSession-like browser seam. Implementations own S256 PKCE and transaction state creation. */
export interface BrowserFlowPort {
  authorize(request: AuthorizationRequest): Promise<AuthorizationResult>;
  endSession(request: EndSessionRequest): Promise<boolean>;
}

export type FetchPort = (input: string, init?: RequestInit) => Promise<Response>;

export interface NativeOidcEnvironment {
  readonly fetch: FetchPort;
  readonly storage: SecureStoragePort;
  readonly browser: BrowserFlowPort;
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

interface RawProviderMetadata {
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly userinfo_endpoint: string;
  readonly end_session_endpoint?: string;
}

interface TokenResponse {
  readonly access_token: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
}

interface ReceivedTokenResponse extends TokenResponse {
  readonly receivedAt: number;
}

type SessionListener = (session: OidcSession | undefined) => void;
type SessionExpiredListener = (event: SessionExpiredEvent) => void;

/** Provider-neutral native OIDC client. It has no React or router dependency. */
export class NativeOidcClient {
  private readonly config: NormalizedConfig;
  private readonly environment: NativeOidcEnvironment;
  private readonly sessionStorageKey: string;
  private readonly forceLoginStorageKey: string;
  private readonly listeners = new Set<SessionListener>();
  private readonly sessionExpiredListeners = new Set<SessionExpiredListener>();
  private currentSession: OidcSession | undefined;
  private initialized = false;
  private initializePromise: Promise<OidcSession | undefined> | undefined;
  private metadataPromise: Promise<OidcProviderMetadata> | undefined;
  private refreshPromise: Promise<string | undefined> | undefined;

  public constructor(config: NativeOidcConfig, environment: NativeOidcEnvironment) {
    this.config = normalizeConfig(config);
    this.environment = environment;
    this.sessionStorageKey = `${this.config.storageKeyPrefix}:session`;
    this.forceLoginStorageKey = `${this.config.storageKeyPrefix}:force-login`;
  }

  /** Loads secure storage once. Corrupt values are removed and treated as signed out. */
  public initialize(): Promise<OidcSession | undefined> {
    if (this.initialized) {
      return Promise.resolve(this.currentSession);
    }
    this.initializePromise ??= this.initializeOnce();
    return this.initializePromise;
  }

  /** Returns the last initialized session without performing I/O. */
  public session(): OidcSession | undefined {
    return this.currentSession;
  }

  public subscribe(listener: SessionListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Observes terminal expiry separately from explicit sign-out or manual clearing. */
  public subscribeSessionExpired(listener: SessionExpiredListener): () => void {
    this.sessionExpiredListeners.add(listener);
    return () => {
      this.sessionExpiredListeners.delete(listener);
    };
  }

  public discover(): Promise<OidcProviderMetadata> {
    this.metadataPromise ??= discoverProvider(this.config.issuer, this.environment.fetch);
    return this.metadataPromise;
  }

  public async signIn(): Promise<SignInResult> {
    await this.initialize();
    const metadata = await this.discover();
    const forceLogin = await this.readForceLogin();
    let result: AuthorizationResult;
    try {
      result = await this.environment.browser.authorize({
        authorizationEndpoint: metadata.authorizationEndpoint,
        clientId: this.config.clientId,
        redirectUri: this.config.redirectUri,
        scopes: this.config.scopes,
        ...(forceLogin ? { prompt: 'login' as const } : {}),
      });
    } catch {
      throw new OidcError('authorization_failed');
    }

    if (result.type === 'cancel' || result.type === 'dismiss') {
      return { status: 'cancelled', reason: result.type };
    }
    if (result.type === 'error' || result.code.length === 0) {
      throw new OidcError('authorization_failed');
    }
    if (result.state.length === 0 || result.state !== result.expectedState) {
      throw new OidcError('callback_state_mismatch');
    }
    if (result.codeVerifier.length === 0) {
      throw new OidcError('pkce_verifier_missing');
    }

    const tokens = await this.exchange(
      metadata.tokenEndpoint,
      new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: this.config.clientId,
        redirect_uri: this.config.redirectUri,
        code: result.code,
        code_verifier: result.codeVerifier,
      }),
      'token_exchange_failed',
    );
    const subject = await fetchSubject(
      metadata.userInfoEndpoint,
      tokens.access_token,
      this.environment.fetch,
    );
    const session = makeSession(tokens, subject);
    await this.writeSession(session);
    await this.deleteForceLoginBestEffort();
    return { status: 'success', subject };
  }

  /** Returns a current token, refreshing once when needed. */
  public async accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    await this.initialize();
    const session = this.currentSession;
    if (session === undefined) {
      return undefined;
    }
    const expiresSoon = session.expiresAt <= this.environment.now() + this.config.refreshLeewayMs;
    if (options.forceRefresh !== true && !expiresSoon) {
      return session.accessToken;
    }
    if (session.refreshToken === undefined) {
      await this.expireSession('refresh_unavailable');
      return undefined;
    }
    if (this.refreshPromise !== undefined) {
      return this.refreshPromise;
    }
    const pending = this.refresh(session);
    this.refreshPromise = pending;
    try {
      return await pending;
    } finally {
      if (this.refreshPromise === pending) {
        this.refreshPromise = undefined;
      }
    }
  }

  /** Clears local state first, then makes a best-effort provider logout attempt. */
  public async signOut(): Promise<SignOutResult> {
    await this.initialize();
    const idToken = this.currentSession?.idToken;
    await this.clearSession();
    await this.setForceLogin();

    let metadata: OidcProviderMetadata;
    try {
      metadata = await this.discover();
    } catch {
      return { providerLogout: 'failed' };
    }
    if (metadata.endSessionEndpoint === undefined) {
      return { providerLogout: 'unavailable' };
    }

    const endpoint = new URL(metadata.endSessionEndpoint);
    endpoint.searchParams.set('client_id', this.config.clientId);
    endpoint.searchParams.set('post_logout_redirect_uri', this.config.postLogoutRedirectUri);
    if (idToken !== undefined) {
      endpoint.searchParams.set('id_token_hint', idToken);
    }
    try {
      const completed = await this.environment.browser.endSession({
        url: endpoint.toString(),
        redirectUri: this.config.postLogoutRedirectUri,
      });
      if (!completed) {
        return { providerLogout: 'failed' };
      }
      await this.deleteForceLoginBestEffort();
      return { providerLogout: 'completed' };
    } catch {
      return { providerLogout: 'failed' };
    }
  }

  /** Clears local authentication state without contacting the provider. */
  public async clearSession(): Promise<void> {
    this.setCurrentSession(undefined);
    try {
      await this.environment.storage.delete(this.sessionStorageKey);
    } catch {
      throw new OidcError('storage_failed');
    }
  }

  private async initializeOnce(): Promise<OidcSession | undefined> {
    let raw: string | null;
    try {
      raw = await this.environment.storage.get(this.sessionStorageKey);
    } catch {
      throw new OidcError('storage_failed');
    }
    const session = parseSession(raw);
    if (raw !== null && session === undefined) {
      this.setCurrentSession(undefined);
      try {
        await this.environment.storage.delete(this.sessionStorageKey);
      } catch {
        throw new OidcError('storage_failed');
      }
    } else {
      this.setCurrentSession(session);
    }
    this.initialized = true;
    return this.currentSession;
  }

  private async refresh(previous: OidcSession): Promise<string | undefined> {
    const refreshToken = previous.refreshToken;
    if (refreshToken === undefined) {
      return undefined;
    }
    let tokens: ReceivedTokenResponse;
    try {
      const metadata = await this.discover();
      tokens = await this.exchange(
        metadata.tokenEndpoint,
        new URLSearchParams({
          grant_type: 'refresh_token',
          client_id: this.config.clientId,
          refresh_token: refreshToken,
          scope: this.config.scopes.join(' '),
        }),
        'refresh_failed',
      );
    } catch (cause) {
      if (isTerminalRefreshError(cause)) {
        if (this.currentSession !== previous) {
          return this.currentSession?.accessToken;
        }
        await this.expireSession('refresh_rejected');
        return undefined;
      }
      throw transientRefreshError(cause);
    }
    if (this.currentSession !== previous) {
      return this.currentSession?.accessToken;
    }
    const session = makeSession(tokens, previous.subject, previous);
    await this.writeSession(session);
    return session.accessToken;
  }

  private async exchange(
    endpoint: string,
    body: URLSearchParams,
    failureCode: 'refresh_failed' | 'token_exchange_failed',
  ): Promise<ReceivedTokenResponse> {
    let response: Response;
    try {
      response = await this.environment.fetch(endpoint, {
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
    let value: unknown;
    try {
      value = await response.json();
    } catch {
      throw new OidcError('invalid_token_response');
    }
    if (!isTokenResponse(value)) {
      throw new OidcError('invalid_token_response');
    }
    return { ...value, receivedAt };
  }

  private async writeSession(session: OidcSession): Promise<void> {
    try {
      await this.environment.storage.set(this.sessionStorageKey, JSON.stringify(session));
    } catch {
      throw new OidcError('storage_failed');
    }
    this.setCurrentSession(session);
  }

  private async readForceLogin(): Promise<boolean> {
    let stored: string | null;
    try {
      stored = await this.environment.storage.get(this.forceLoginStorageKey);
    } catch {
      throw new OidcError('storage_failed');
    }
    if (stored === null || stored === '0') {
      return false;
    }
    if (stored !== '1') {
      await this.setForceLogin();
    }
    return true;
  }

  private async setForceLogin(): Promise<void> {
    try {
      await this.environment.storage.set(this.forceLoginStorageKey, '1');
    } catch {
      throw new OidcError('storage_failed');
    }
  }

  private async deleteForceLoginBestEffort(): Promise<void> {
    try {
      await this.environment.storage.delete(this.forceLoginStorageKey);
    } catch {
      // Retaining prompt=login is the fail-safe outcome when storage deletion fails.
    }
  }

  private setCurrentSession(session: OidcSession | undefined): void {
    this.currentSession = session;
    for (const listener of this.listeners) {
      try {
        listener(session);
      } catch {
        // Observers cannot interrupt persistence or authentication transitions.
      }
    }
  }

  private async expireSession(reason: SessionExpiredReason): Promise<void> {
    await this.clearSession();
    const event: SessionExpiredEvent = { type: 'session-expired', reason };
    for (const listener of this.sessionExpiredListeners) {
      try {
        listener(event);
      } catch {
        // Observers cannot change the terminal unauthenticated result.
      }
    }
  }
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

/** Returns an allowlisted message without reflecting untrusted error content. */
export function safeAuthErrorMessage(cause: unknown): string {
  return cause instanceof OidcError ? SAFE_ERROR_MESSAGES[cause.code] : 'OIDC login failed.';
}

async function discoverProvider(
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

async function fetchSubject(
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
  if (!isObject(value) || typeof value['sub'] !== 'string' || value['sub'].length === 0) {
    throw new OidcError('invalid_userinfo_response');
  }
  return value['sub'];
}

function normalizeConfig(config: NativeOidcConfig): NormalizedConfig {
  const issuer = normalizeIssuer(config.issuer);
  const clientId = requiredText(config.clientId, 'OIDC client ID');
  const redirectUri = requiredUrl(config.redirectUri, 'OIDC redirect URI');
  const refreshLeewaySeconds = config.refreshLeewaySeconds ?? 30;
  if (!Number.isFinite(refreshLeewaySeconds) || refreshLeewaySeconds < 0) {
    throw new RangeError('OIDC refresh leeway must be a non-negative number.');
  }
  const storageKeyPrefix =
    config.storageKeyPrefix ?? `@baukit/auth-native:${encodeURIComponent(issuer)}:${clientId}`;
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

function makeSession(
  tokens: ReceivedTokenResponse,
  subject: string,
  previous?: OidcSession,
): OidcSession {
  const refreshToken = tokens.refresh_token ?? previous?.refreshToken;
  const idToken = tokens.id_token ?? previous?.idToken;
  return Object.freeze({
    subject,
    accessToken: tokens.access_token,
    expiresAt: tokens.receivedAt + tokens.expires_in * 1000,
    ...(refreshToken === undefined ? {} : { refreshToken }),
    ...(idToken === undefined ? {} : { idToken }),
  });
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

function parseSession(raw: string | null): OidcSession | undefined {
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

function isSession(value: unknown): value is OidcSession {
  return (
    isObject(value) &&
    typeof value['subject'] === 'string' &&
    value['subject'].length > 0 &&
    typeof value['accessToken'] === 'string' &&
    value['accessToken'].length > 0 &&
    typeof value['expiresAt'] === 'number' &&
    Number.isFinite(value['expiresAt']) &&
    (value['refreshToken'] === undefined ||
      (typeof value['refreshToken'] === 'string' && value['refreshToken'].length > 0)) &&
    (value['idToken'] === undefined ||
      (typeof value['idToken'] === 'string' && value['idToken'].length > 0))
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
    (value['refresh_token'] === undefined ||
      (typeof value['refresh_token'] === 'string' && value['refresh_token'].length > 0)) &&
    (value['id_token'] === undefined ||
      (typeof value['id_token'] === 'string' && value['id_token'].length > 0))
  );
}

function isProviderMetadata(value: unknown): value is RawProviderMetadata {
  if (
    !isObject(value) ||
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

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
