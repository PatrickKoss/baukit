export interface OidcClientConfig {
  readonly issuer: string;
  readonly clientId: string;
  readonly redirectUri: string;
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

const tokenStorageKey = '{{ context.app_name }}:oidc:tokens';
const transactionStorageKey = '{{ context.app_name }}:oidc:transaction';

export class OidcClient {
  public constructor(private readonly config: OidcClientConfig) {}

  public hasSession(): boolean {
    return this.readTokens() !== undefined;
  }

  public async login(): Promise<void> {
    const state = randomUrlSafe(32);
    const verifier = randomUrlSafe(64);
    const challenge = base64UrlEncode(
      new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))),
    );
    sessionStorage.setItem(transactionStorageKey, JSON.stringify({ state, verifier }));
    window.location.assign(buildAuthorizationUrl(this.config, { state, challenge }).toString());
  }

  public async handleCallback(): Promise<boolean> {
    const callback = new URL(window.location.href);
    const code = callback.searchParams.get('code');
    const returnedState = callback.searchParams.get('state');
    if (code === null && returnedState === null) {
      return false;
    }
    const transaction = readTransaction(sessionStorage.getItem(transactionStorageKey));
    sessionStorage.removeItem(transactionStorageKey);
    if (code === null || returnedState === null || transaction?.state !== returnedState) {
      throw new Error('OIDC callback state did not match the login transaction.');
    }
    await this.exchange(
      new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: this.config.clientId,
        redirect_uri: this.config.redirectUri,
        code,
        code_verifier: transaction.verifier,
      }),
    );
    callback.searchParams.delete('code');
    callback.searchParams.delete('state');
    callback.searchParams.delete('session_state');
    window.history.replaceState({}, '', callback);
    return true;
  }

  public async accessToken(): Promise<string | undefined> {
    const tokens = this.readTokens();
    if (tokens === undefined) {
      return undefined;
    }
    if (tokens.expiresAt > Date.now() + 30_000) {
      return tokens.accessToken;
    }
    if (tokens.refreshToken === undefined) {
      this.clear();
      return undefined;
    }
    return (
      await this.exchange(
        new URLSearchParams({
          grant_type: 'refresh_token',
          client_id: this.config.clientId,
          refresh_token: tokens.refreshToken,
        }),
        tokens.refreshToken,
      )
    ).accessToken;
  }

  public logout(): void {
    const idToken = this.readTokens()?.idToken;
    this.clear();
    const endpoint = new URL(`${this.config.issuer}/protocol/openid-connect/logout`);
    endpoint.searchParams.set('client_id', this.config.clientId);
    endpoint.searchParams.set('post_logout_redirect_uri', this.config.redirectUri);
    if (idToken !== undefined) {
      endpoint.searchParams.set('id_token_hint', idToken);
    }
    window.location.assign(endpoint.toString());
  }

  private async exchange(
    body: URLSearchParams,
    existingRefreshToken?: string,
  ): Promise<StoredTokens> {
    const response = await fetch(`${this.config.issuer}/protocol/openid-connect/token`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body,
    });
    if (!response.ok) {
      throw new Error(`OIDC token exchange failed with HTTP ${String(response.status)}.`);
    }
    const payload: unknown = await response.json();
    if (!isTokenResponse(payload)) {
      throw new TypeError('OIDC token endpoint returned an invalid response.');
    }
    const refreshToken = payload.refresh_token ?? existingRefreshToken;
    const tokens: StoredTokens = {
      accessToken: payload.access_token,
      expiresAt: Date.now() + payload.expires_in * 1000,
      ...(refreshToken === undefined ? {} : { refreshToken }),
      ...(payload.id_token === undefined ? {} : { idToken: payload.id_token }),
    };
    localStorage.setItem(tokenStorageKey, JSON.stringify(tokens));
    return tokens;
  }

  private readTokens(): StoredTokens | undefined {
    const raw = localStorage.getItem(tokenStorageKey);
    if (raw === null) {
      return undefined;
    }
    try {
      const value: unknown = JSON.parse(raw);
      return isStoredTokens(value) ? value : undefined;
    } catch {
      return undefined;
    }
  }

  private clear(): void {
    localStorage.removeItem(tokenStorageKey);
    sessionStorage.removeItem(transactionStorageKey);
  }
}

export function buildAuthorizationUrl(
  config: OidcClientConfig,
  pkce: { readonly state: string; readonly challenge: string },
): URL {
  const endpoint = new URL(`${config.issuer}/protocol/openid-connect/auth`);
  endpoint.search = new URLSearchParams({
    client_id: config.clientId,
    redirect_uri: config.redirectUri,
    response_type: 'code',
    scope: 'openid profile email offline_access',
    state: pkce.state,
    code_challenge: pkce.challenge,
    code_challenge_method: 'S256',
  }).toString();
  return endpoint;
}

function randomUrlSafe(byteLength: number): string {
  return base64UrlEncode(crypto.getRandomValues(new Uint8Array(byteLength)));
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
    return typeof value === 'object' && value !== null &&
      typeof (value as Record<string, unknown>)['state'] === 'string' &&
      typeof (value as Record<string, unknown>)['verifier'] === 'string'
      ? (value as LoginTransaction)
      : undefined;
  } catch {
    return undefined;
  }
}

function isStoredTokens(value: unknown): value is StoredTokens {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as Record<string, unknown>)['accessToken'] === 'string' &&
    typeof (value as Record<string, unknown>)['expiresAt'] === 'number'
  );
}

function isTokenResponse(value: unknown): value is {
  readonly access_token: string;
  readonly expires_in: number;
  readonly refresh_token?: string;
  readonly id_token?: string;
} {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as Record<string, unknown>)['access_token'] === 'string' &&
    typeof (value as Record<string, unknown>)['expires_in'] === 'number' &&
    (typeof (value as Record<string, unknown>)['refresh_token'] === 'string' ||
      (value as Record<string, unknown>)['refresh_token'] === undefined) &&
    (typeof (value as Record<string, unknown>)['id_token'] === 'string' ||
      (value as Record<string, unknown>)['id_token'] === undefined)
  );
}

const configuredIssuer: unknown = import.meta.env['VITE_OIDC_ISSUER'];
const configuredClientId: unknown = import.meta.env['VITE_OIDC_CLIENT_ID'];

export const authClient = new OidcClient({
  issuer:
    typeof configuredIssuer === 'string'
      ? configuredIssuer.replace(/\/$/u, '')
      : 'http://localhost:8081/realms/{{ context.app_name }}',
  clientId:
    typeof configuredClientId === 'string' ? configuredClientId : '{{ context.app_name }}-web',
  redirectUri: `${typeof window === 'undefined' ? 'http://localhost:5173' : window.location.origin}/`,
});
