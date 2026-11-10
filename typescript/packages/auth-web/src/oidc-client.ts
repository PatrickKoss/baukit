import {
  authorizationPrompt,
  decoratedAuthorizationState,
  type LoginOptions,
} from './login-options.js';
import { OidcError } from './oidc-errors.js';
import {
  base64UrlEncode,
  buildAuthorizationUrl,
  normalizeIssuer,
  normalizeScopes,
  randomUrlSafe,
  requiredText,
  requiredUrl,
} from './oidc-helpers.js';
import { discoverProvider, readTransaction } from './oidc-validation.js';

export interface OidcClientConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly audience?: string;
  readonly resource?: string;
  /** `openid` is prepended when omitted. Defaults to `openid profile email`. */
  readonly scopes?: readonly string[];
  /** Adds `offline_access` to the requested scopes. Defaults to false. */
  readonly offlineAccess?: boolean;
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

export interface PkceAuthorizationRequest {
  readonly state: string;
  readonly challenge: string;
}

/** The verified authorization response, ready for the backend to redeem. */
export interface AuthorizationCallback {
  readonly code: string;
  readonly codeVerifier: string;
  readonly redirectUri: string;
}

interface NormalizedConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
  readonly audience?: string;
  readonly resource?: string;
  readonly scopes: readonly string[];
  readonly storageKeyPrefix: string;
}

const CALLBACK_PARAMETERS = [
  'code',
  'state',
  'session_state',
  'iss',
  'error',
  'error_description',
  'error_uri',
] as const;

/**
 * Browser OIDC authorization-code redirect with S256 PKCE. It never redeems the
 * code and never holds tokens; `BrowserSessionClient` hands the code to the backend.
 */
export class OidcClient {
  public readonly storageKeyPrefix: string;
  private readonly config: NormalizedConfig;
  private readonly environment: OidcClientEnvironment;
  private readonly transactionStorageKey: string;
  private readonly forceLoginStorageKey: string;
  private metadataPromise: Promise<OidcProviderMetadata> | undefined;

  public constructor(
    config: OidcClientConfig,
    environment: OidcClientEnvironment = browserOidcEnvironment(),
  ) {
    this.config = normalizeConfig(config);
    this.environment = environment;
    this.storageKeyPrefix = this.config.storageKeyPrefix;
    this.transactionStorageKey = `${this.storageKeyPrefix}:transaction`;
    this.forceLoginStorageKey = `${this.storageKeyPrefix}:force-login`;
  }

  public discover(): Promise<OidcProviderMetadata> {
    this.metadataPromise ??= discoverProvider(this.config.issuer, this.environment.fetch);
    return this.metadataPromise;
  }

  /** Stores the PKCE transaction in sessionStorage and navigates to the provider. */
  public async login(options: LoginOptions = {}): Promise<void> {
    const metadata = await this.discover();
    const state = this.authorizationState(options);
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
      this.config,
      metadata.authorizationEndpoint,
      { state, challenge },
      authorizationPrompt(
        options.entry,
        this.environment.localStorage.getItem(this.forceLoginStorageKey) !== null,
      ),
    );
    this.environment.navigate(authorizationUrl.toString());
  }

  private authorizationState(options: LoginOptions): string {
    if (options.stateDecoration === undefined) {
      return randomUrlSafe(this.environment.crypto, 32);
    }
    return decoratedAuthorizationState(
      options.stateDecoration,
      this.environment.crypto.getRandomValues(new Uint8Array(32)),
    );
  }

  /**
   * Strips the callback parameters from the URL and checks state and issuer.
   * Returns `undefined` when the current URL is not a callback.
   */
  public completeCallback(): AuthorizationCallback | undefined {
    const callback = new URL(this.environment.currentUrl());
    if (!hasCallbackParameters(callback)) {
      return undefined;
    }
    const code = callback.searchParams.get('code');
    const returnedState = callback.searchParams.get('state');
    const returnedIssuer = callback.searchParams.get('iss');
    const providerError = callback.searchParams.has('error');
    for (const parameter of CALLBACK_PARAMETERS) {
      callback.searchParams.delete(parameter);
    }
    this.environment.replaceUrl(callback.toString());

    const transaction = readTransaction(
      this.environment.sessionStorage.getItem(this.transactionStorageKey),
    );
    this.clearTransaction();
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
    return { code, codeVerifier: transaction.verifier, redirectUri: this.config.redirectUri };
  }

  /** Makes the next `login()` request `prompt=login`, surviving reloads. */
  public requireFreshLogin(): void {
    this.environment.localStorage.setItem(this.forceLoginStorageKey, '1');
  }

  /** Drops the fresh-login requirement after a successful sign-in. */
  public confirmFreshLogin(): void {
    this.environment.localStorage.removeItem(this.forceLoginStorageKey);
  }

  public clearTransaction(): void {
    this.environment.sessionStorage.removeItem(this.transactionStorageKey);
  }
}

function normalizeConfig(config: OidcClientConfig): NormalizedConfig {
  const issuer = normalizeIssuer(config.issuer);
  const clientId = requiredText(config.clientId, 'OIDC client ID');
  const storageKeyPrefix =
    config.storageKeyPrefix ?? `@baukit/auth-web:${encodeURIComponent(issuer)}:${clientId}`;
  if (storageKeyPrefix.length === 0) {
    throw new TypeError('OIDC storage key prefix must not be empty.');
  }
  return {
    issuer,
    clientId,
    redirectUri: requiredUrl(config.redirectUri, 'OIDC redirect URI'),
    ...(config.audience === undefined
      ? {}
      : { audience: requiredText(config.audience, 'OIDC audience') }),
    ...(config.resource === undefined
      ? {}
      : { resource: requiredText(config.resource, 'OIDC resource') }),
    scopes: normalizeScopes(config.scopes, config.offlineAccess),
    storageKeyPrefix,
  };
}

function hasCallbackParameters(url: URL): boolean {
  return ['code', 'state', 'error', 'iss', 'session_state'].some((name) =>
    url.searchParams.has(name),
  );
}

export function browserOidcEnvironment(): OidcClientEnvironment {
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
