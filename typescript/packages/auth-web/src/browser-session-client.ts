import {
  browserSessionEnvironment,
  isIssuedSession,
  readAccountHint,
  readEndSessionUrl,
  readSessionMessage,
  type AccountHint,
  type BrowserSessionEnvironment,
  type IssuedSession,
  type SessionChannel,
  type SessionMessage,
  type SignedOutCause,
} from './browser-session-support.js';
import { OidcClient, type OidcClientConfig } from './oidc-client.js';
import { OidcError } from './oidc-errors.js';
import { requiredUrl } from './oidc-helpers.js';
import type {
  ClearSessionOptions,
  SessionExpiredEvent,
  SessionExpiredListener,
  SessionExpiredReason,
} from './session-events.js';

export interface BrowserSessionClientConfig extends OidcClientConfig {
  /** Origin (and optional base path) of the backend that mounts `/auth/session`. */
  readonly apiBaseUrl: string;
  /** Defaults to 30 seconds. */
  readonly refreshLeewaySeconds?: number;
}

export const BROWSER_SESSION_PATH = '/auth/session';
export const BROWSER_SESSION_HEADER = 'X-Baukit-Session';

const DEFAULT_REFRESH_LEEWAY_SECONDS = 30;
const MS_PER_SECOND = 1000;
const HTTP_NO_CONTENT = 204;
const HTTP_UNAUTHORIZED = 401;
const HTTP_TOO_MANY_REQUESTS = 429;
const HTTP_SERVER_ERROR = 500;
/** How long a tab that waited for the refresh lock waits for the winner's token. */
export const SESSION_HANDOFF_TIMEOUT_MS = 2000;

type SessionEndpoint = 'exchange' | 'refresh' | 'logout';

interface MemoryToken extends IssuedSession {
  readonly receivedAt: number;
}

/**
 * Browser sign-in against the product backend's `/auth/session` routes. The
 * backend holds the refresh token behind an HttpOnly cookie; the access token
 * lives only in this object's memory.
 */
export class BrowserSessionClient {
  private readonly oidc: OidcClient;
  private readonly environment: BrowserSessionEnvironment;
  private readonly sessionBaseUrl: string;
  private readonly refreshLeewayMs: number;
  private readonly accountStorageKey: string;
  private readonly lockName: string;
  private readonly generationStorageKey: string;
  private readonly sessionWaiters = new Set<() => void>();
  private readonly channel: SessionChannel | undefined;
  private readonly listeners = new Set<SessionExpiredListener>();
  private token: MemoryToken | undefined;
  private callbackPromise: Promise<boolean> | undefined;
  private refreshPromise: Promise<string | undefined> | undefined;
  private revision = 0;

  public constructor(
    config: BrowserSessionClientConfig,
    environment: BrowserSessionEnvironment = browserSessionEnvironment(),
  ) {
    this.oidc = new OidcClient(config, environment);
    this.environment = environment;
    this.sessionBaseUrl = `${requiredUrl(config.apiBaseUrl, 'API base URL').replace(/\/+$/u, '')}${BROWSER_SESSION_PATH}`;
    this.refreshLeewayMs = refreshLeewayMs(config.refreshLeewaySeconds);
    const prefix = this.oidc.storageKeyPrefix;
    this.accountStorageKey = `${prefix}:account`;
    this.lockName = `${prefix}:refresh`;
    this.generationStorageKey = `${prefix}:refresh-generation`;
    this.channel = environment.openChannel?.(`${prefix}:session`);
    this.channel?.subscribe((message) => {
      this.receive(readSessionMessage(message));
    });
  }

  /** True while a token is in memory or the account hint says this browser signed in. */
  public hasSession(): boolean {
    return this.token !== undefined || this.account() !== undefined;
  }

  /** The signed-in account, readable offline. */
  public account(): AccountHint | undefined {
    return readAccountHint(this.environment.localStorage.getItem(this.accountStorageKey));
  }

  public login(): Promise<void> {
    return this.oidc.login();
  }

  /** Returns one shared promise so repeated effects cannot redeem a code twice. */
  public handleCallback(): Promise<boolean> {
    this.callbackPromise ??= this.handleCallbackOnce();
    return this.callbackPromise;
  }

  /** Refreshes once when an account hint exists, as on page load. */
  public async restore(): Promise<boolean> {
    return (await this.getAccessToken()) !== undefined;
  }

  /**
   * Resolves the in-memory access token, refreshing it near expiry or when
   * forced. A network failure rejects with a retryable `OidcError` and keeps
   * the account hint; only a 401 signs out.
   */
  public getAccessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    const token = this.token;
    if (token !== undefined && options.forceRefresh !== true && this.isFresh(token)) {
      return Promise.resolve(token.accessToken);
    }
    if (token === undefined && this.account() === undefined) {
      return Promise.resolve(undefined);
    }
    this.refreshPromise ??= this.coordinatedRefresh(this.revision).finally(() => {
      this.refreshPromise = undefined;
    });
    return this.refreshPromise;
  }

  public accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    return this.getAccessToken(options);
  }

  /** Observes sign-out by expiry, erasure or another tab, not this tab's own `clearSession()`. */
  public onSessionExpired(listener: SessionExpiredListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  public subscribeSessionExpired(listener: SessionExpiredListener): () => void {
    return this.onSessionExpired(listener);
  }

  /**
   * Signs out every tab, ends the backend session and navigates to the IdP
   * end-session URL when the backend returns one.
   */
  public async logout(): Promise<boolean> {
    this.clearSession();
    let response: Response;
    try {
      response = await this.post('logout');
    } catch {
      throw new OidcError('logout_failed', { retryable: true });
    }
    if (response.status === HTTP_NO_CONTENT) {
      return false;
    }
    if (!response.ok) {
      throw new OidcError('logout_failed', { status: response.status });
    }
    const endSessionUrl = readEndSessionUrl(await readJson(response));
    if (endSessionUrl === undefined) {
      return false;
    }
    this.environment.navigate(endSessionUrl);
    return true;
  }

  /** Clears local state in every tab without contacting the backend. */
  public clearSession(options: ClearSessionOptions = {}): void {
    if (options.reason !== undefined) {
      this.expire(options.reason);
      return;
    }
    this.clearLocal();
    this.broadcast({ type: 'signed-out', cause: 'cleared' });
  }

  public dispose(): void {
    this.channel?.close();
  }

  private async handleCallbackOnce(): Promise<boolean> {
    const callback = this.oidc.completeCallback();
    if (callback === undefined) {
      return false;
    }
    const revision = this.revision;
    let response: Response;
    try {
      response = await this.post('exchange', callback);
    } catch {
      throw new OidcError('token_exchange_failed');
    }
    if (!response.ok) {
      throw new OidcError('token_exchange_failed', { status: response.status });
    }
    const session = await readIssuedSession(response);
    if (this.revision === revision) {
      this.accept(session, { newSignIn: true });
      this.oidc.confirmFreshLogin();
    }
    return true;
  }

  private coordinatedRefresh(requestedAt: number): Promise<string | undefined> {
    const generation = this.refreshGeneration();
    const run = () => this.refreshUnlessSettled(requestedAt, generation);
    const locks = this.environment.locks;
    return locks === undefined ? run() : locks.request(this.lockName, run);
  }

  /** Skips the request when another tab refreshed or signed out while this one waited. */
  private async refreshUnlessSettled(
    requestedAt: number,
    generation: number,
  ): Promise<string | undefined> {
    if (this.revision === requestedAt && this.refreshGeneration() !== generation) {
      await this.nextSession();
    }
    const current = this.token;
    if (this.revision !== requestedAt) {
      if (current === undefined) {
        return undefined;
      }
      if (this.isFresh(current)) {
        return current.accessToken;
      }
    }
    const revision = this.revision;
    let response: Response;
    try {
      response = await this.post('refresh');
    } catch {
      throw new OidcError('refresh_failed', { retryable: true });
    }
    if (response.status === HTTP_UNAUTHORIZED) {
      if (this.revision === revision) {
        this.expire('refresh_rejected');
      }
      return this.token?.accessToken;
    }
    if (!response.ok) {
      throw new OidcError('refresh_failed', {
        status: response.status,
        retryable: isRetryableStatus(response.status),
      });
    }
    const session = await readIssuedSession(response);
    if (this.revision !== revision) {
      return this.token?.accessToken;
    }
    this.accept(session, { newSignIn: false });
    return session.accessToken;
  }

  private accept(session: IssuedSession, options: { readonly newSignIn: boolean }): void {
    this.remember(session);
    this.environment.localStorage.setItem(
      this.generationStorageKey,
      String(this.refreshGeneration() + 1),
    );
    const hint = this.account();
    if (options.newSignIn || hint?.subject !== session.subject) {
      const next: AccountHint = { subject: session.subject, signedInAt: this.environment.now() };
      this.environment.localStorage.setItem(this.accountStorageKey, JSON.stringify(next));
    }
    this.broadcast({
      type: 'session',
      accessToken: session.accessToken,
      expiresAt: session.expiresAt,
      subject: session.subject,
    });
  }

  private remember(session: IssuedSession): void {
    this.revision += 1;
    this.token = {
      accessToken: session.accessToken,
      expiresAt: session.expiresAt,
      subject: session.subject,
      receivedAt: this.environment.now(),
    };
  }

  private receive(message: SessionMessage | undefined): void {
    if (message === undefined) {
      return;
    }
    if (message.type === 'session') {
      this.remember(message);
      this.releaseSessionWaiters();
      return;
    }
    this.revision += 1;
    this.token = undefined;
    this.releaseSessionWaiters();
    this.notify(expiredReason(message.cause));
  }

  /** Counts sessions issued in any tab; it holds no token or expiry. */
  private refreshGeneration(): number {
    const value = Number(this.environment.localStorage.getItem(this.generationStorageKey));
    return Number.isSafeInteger(value) ? value : 0;
  }

  /** Resolves when another tab's session message arrives or the handoff times out. */
  private nextSession(): Promise<void> {
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.sessionWaiters.delete(done);
        resolve();
      };
      const timer = setTimeout(done, SESSION_HANDOFF_TIMEOUT_MS);
      this.sessionWaiters.add(done);
    });
  }

  private releaseSessionWaiters(): void {
    for (const done of [...this.sessionWaiters]) {
      done();
    }
  }

  private expire(reason: SessionExpiredReason): void {
    this.clearLocal();
    if (reason === 'profile_erased') {
      this.oidc.requireFreshLogin();
    }
    this.broadcast({ type: 'signed-out', cause: reason });
    this.notify(reason);
  }

  private clearLocal(): void {
    this.revision += 1;
    this.token = undefined;
    this.environment.localStorage.removeItem(this.accountStorageKey);
    this.oidc.clearTransaction();
  }

  private notify(reason: SessionExpiredReason): void {
    const event: SessionExpiredEvent = { type: 'session-expired', reason };
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch {
        // Observers cannot change the terminal unauthenticated result.
      }
    }
  }

  private broadcast(message: SessionMessage): void {
    this.channel?.postMessage(message);
  }

  private isFresh(token: MemoryToken): boolean {
    const window = Math.min(this.refreshLeewayMs, (token.expiresAt - token.receivedAt) / 2);
    return token.expiresAt > this.environment.now() + window;
  }

  private post(endpoint: SessionEndpoint, body?: object): Promise<Response> {
    return this.environment.fetch(`${this.sessionBaseUrl}/${endpoint}`, {
      method: 'POST',
      credentials: 'include',
      cache: 'no-store',
      headers:
        body === undefined
          ? { [BROWSER_SESSION_HEADER]: '1' }
          : { [BROWSER_SESSION_HEADER]: '1', 'content-type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  }
}

function refreshLeewayMs(seconds = DEFAULT_REFRESH_LEEWAY_SECONDS): number {
  if (!Number.isFinite(seconds) || seconds < 0) {
    throw new RangeError('OIDC refresh leeway must be a non-negative number.');
  }
  return seconds * MS_PER_SECOND;
}

function isRetryableStatus(status: number): boolean {
  return status === HTTP_TOO_MANY_REQUESTS || status >= HTTP_SERVER_ERROR;
}

function expiredReason(cause: SignedOutCause): SessionExpiredReason {
  return cause === 'cleared' ? 'signed_out' : cause;
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return (await response.json()) as unknown;
  } catch {
    return undefined;
  }
}

async function readIssuedSession(response: Response): Promise<IssuedSession> {
  const value = await readJson(response);
  if (!isIssuedSession(value)) {
    throw new OidcError('invalid_token_response');
  }
  return value;
}
