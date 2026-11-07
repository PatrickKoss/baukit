import { OidcError } from './oidc-errors.js';
import {
  discoverProvider,
  fetchSubject,
  normalizeConfig,
  refreshWindowMs,
  makeSession,
  readProviderError,
  isTerminalRefreshRejection,
  isTerminalRefreshError,
  transientRefreshError,
  parseSession,
  isTokenResponse,
  type NormalizedConfig,
  type ReceivedTokenResponse,
} from './oidc-support.js';

export {
  AUTHORIZATION_STATE_ENTROPY_BYTES,
  appearanceStateDecoration,
  decoratedAuthorizationState,
  type AppearanceMode,
  type AppearanceState,
} from './authorization-state.js';
export { OidcError, safeAuthErrorMessage, type OidcErrorCode } from './oidc-errors.js';
export { normalizeIssuer } from './oidc-support.js';

export interface NativeOidcConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly audience?: string;
  readonly resource?: string;
  /** `openid` is prepended when omitted. Defaults to `openid profile email`. */
  readonly scopes?: readonly string[];
  /** Adds `offline_access` to the requested scopes. Defaults to false. */
  readonly offlineAccess?: boolean;
  /** Defaults to `redirectUri`. */
  readonly postLogoutRedirectUri?: string;
  /** Defaults to 30 seconds. */
  readonly refreshLeewaySeconds?: number;
  /** Refreshes an expired stored session during `initialize()`. Defaults to false. */
  readonly refreshOnRestore?: boolean;
  /** Overrides the issuer/client-specific prefix. Only ASCII letters, digits, '.', '-' and '_' are allowed. */
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
  readonly issuedAt: number;
  readonly expiresAt: number;
}

export interface SignInOptions {
  /** Forwarded to the browser flow, for example `appearanceStateDecoration({ mode: 'dark' })`. */
  readonly stateDecoration?: readonly string[];
}

export type SignInResult =
  | { readonly status: 'success'; readonly subject: string }
  | { readonly status: 'cancelled'; readonly reason: 'cancel' | 'dismiss' };

export type SignOutResult =
  | { readonly providerLogout: 'completed' }
  | { readonly providerLogout: 'unavailable' }
  | { readonly providerLogout: 'failed' };

export type SessionExpiredReason = 'refresh_rejected' | 'refresh_unavailable' | 'profile_erased';

export interface ClearSessionOptions {
  readonly reason?: 'profile_erased';
}

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
  readonly audience?: string;
  readonly resource?: string;
  readonly prompt?: 'login';
  /** Segments a decorating browser flow puts in front of its random state. Other flows ignore them. */
  readonly stateDecoration?: readonly string[];
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

export interface NativeTokenProtocol {
  discover(): Promise<OidcProviderMetadata>;
  exchange(endpoint: string, body: URLSearchParams): Promise<Response>;
  subject(accessToken: string): Promise<string>;
  logout?(session: OidcSession, redirectUri: string): Promise<boolean>;
}

export interface NativeOidcEnvironment {
  readonly protocol?: NativeTokenProtocol;
  readonly fetch: FetchPort;
  readonly storage: SecureStoragePort;
  readonly browser: BrowserFlowPort;
  readonly now: () => number;
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
    this.sessionStorageKey = `${this.config.storageKeyPrefix}.session`;
    this.forceLoginStorageKey = `${this.config.storageKeyPrefix}.force-login`;
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
    this.metadataPromise ??=
      this.environment.protocol?.discover() ??
      discoverProvider(this.config.issuer, this.environment.fetch);
    return this.metadataPromise;
  }

  public async signIn(options: SignInOptions = {}): Promise<SignInResult> {
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
        ...(this.config.audience === undefined ? {} : { audience: this.config.audience }),
        ...(this.config.resource === undefined ? {} : { resource: this.config.resource }),
        ...(forceLogin ? { prompt: 'login' as const } : {}),
        ...(options.stateDecoration === undefined
          ? {}
          : { stateDecoration: options.stateDecoration }),
      });
    } catch {
      throw new OidcError('authorization_failed');
    }

    if (result.type === 'cancel' || result.type === 'dismiss') {
      return { status: 'cancelled', reason: result.type };
    }
    assertAuthorizationSucceeded(result);

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
    const subject = await (this.environment.protocol?.subject(tokens.access_token) ??
      fetchSubject(metadata.userInfoEndpoint, tokens.access_token, this.environment.fetch));
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
    const leewayMs = refreshWindowMs(
      session.issuedAt,
      session.expiresAt,
      this.config.refreshLeewayMs,
    );
    const expiresSoon = session.expiresAt <= this.environment.now() + leewayMs;
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
    const session = this.currentSession;
    const idToken = session?.idToken;
    await this.clearSession();
    await this.setForceLogin();

    if (session !== undefined && this.environment.protocol?.logout !== undefined) {
      try {
        const completed = await this.environment.protocol.logout(
          session,
          this.config.postLogoutRedirectUri,
        );
        if (completed) await this.deleteForceLoginBestEffort();
        return { providerLogout: completed ? 'completed' : 'failed' };
      } catch {
        return { providerLogout: 'failed' };
      }
    }

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
  public async clearSession(options: ClearSessionOptions = {}): Promise<void> {
    if (options.reason !== undefined) {
      await this.expireSession(options.reason);
      return;
    }
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
    if (this.config.refreshOnRestore) await this.refreshExpiredRestore();
    return this.currentSession;
  }

  private async refreshExpiredRestore(): Promise<void> {
    const session = this.currentSession;
    if (session === undefined || session.expiresAt > this.environment.now()) return;
    try {
      await this.accessToken();
    } catch (cause) {
      if (!(cause instanceof OidcError && cause.retryable)) throw cause;
    }
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
      response = await (this.environment.protocol?.exchange(endpoint, body) ??
        this.environment.fetch(endpoint, {
          method: 'POST',
          headers: { 'content-type': 'application/x-www-form-urlencoded' },
          body,
        }));
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
    if (reason === 'profile_erased') await this.setForceLogin();
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

function assertAuthorizationSucceeded(
  result: AuthorizationResult,
): asserts result is Extract<AuthorizationResult, { readonly type: 'success' }> {
  if (result.type !== 'success' || result.code.length === 0) {
    throw new OidcError('authorization_failed');
  }
  if (result.state.length === 0 || result.state !== result.expectedState) {
    throw new OidcError('callback_state_mismatch');
  }
  if (result.codeVerifier.length === 0) {
    throw new OidcError('pkce_verifier_missing');
  }
}
