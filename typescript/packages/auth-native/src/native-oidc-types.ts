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
