import {
  OidcError,
  type OidcSession,
  type SessionExpiredEvent,
  type SignInResult,
  type SignOutResult,
} from './index.js';
import { tokenTiming } from './workos.js';

export interface ClerkNativePort {
  subject(): string | undefined;
  getToken(skipCache: boolean): Promise<string | null>;
  signIn(): Promise<boolean>;
  signOut(): Promise<void>;
}

/** Keeps SDK session transitions behind the native authentication contract. */
export class ClerkNativeClient {
  private port: ClerkNativePort | undefined;
  private current: OidcSession | undefined;
  private cleared = false;
  private readonly listeners = new Set<(session: OidcSession | undefined) => void>();
  private readonly expired = new Set<(event: SessionExpiredEvent) => void>();
  private readonly ready: Promise<void>;
  private resolveReady: (() => void) | undefined;

  public constructor() {
    this.ready = new Promise((resolve) => {
      this.resolveReady = resolve;
    });
  }

  public bind(port: ClerkNativePort): void {
    this.port = port;
    this.resolveReady?.();
    this.resolveReady = undefined;
  }

  public async initialize(): Promise<OidcSession | undefined> {
    await this.accessToken();
    return this.current;
  }

  public session(): OidcSession | undefined {
    return this.current;
  }

  public subscribe(listener: (session: OidcSession | undefined) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  public subscribeSessionExpired(listener: (event: SessionExpiredEvent) => void): () => void {
    this.expired.add(listener);
    return () => {
      this.expired.delete(listener);
    };
  }

  public async accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    await this.ready;
    const port = this.port;
    if (this.cleared || port?.subject() === undefined) {
      this.setSession(undefined);
      return undefined;
    }
    const token = await port.getToken(options.forceRefresh ?? false);
    if (token === null) {
      const hadSession = this.current !== undefined;
      await this.clearSession();
      if (hadSession)
        for (const listener of this.expired)
          listener({ type: 'session-expired', reason: 'refresh_rejected' });
      return undefined;
    }
    const timing = tokenTiming(token);
    if (timing.subject !== port.subject()) throw new OidcError('invalid_token_response');
    this.setSession({ subject: timing.subject, accessToken: token, expiresAt: timing.expiresAt });
    return token;
  }

  public async signIn(): Promise<SignInResult> {
    await this.ready;
    if (this.port === undefined) throw new OidcError('authorization_failed');
    if (!(await this.port.signIn())) return { status: 'cancelled', reason: 'cancel' };
    this.cleared = false;
    await this.accessToken({ forceRefresh: true });
    if (this.current === undefined) throw new OidcError('authorization_failed');
    return { status: 'success', subject: this.current.subject };
  }

  public clearSession(): Promise<void> {
    this.cleared = true;
    this.setSession(undefined);
    return Promise.resolve();
  }

  public async signOut(): Promise<SignOutResult> {
    await this.clearSession();
    await this.ready;
    if (this.port === undefined) return { providerLogout: 'unavailable' };
    try {
      await this.port.signOut();
      return { providerLogout: 'completed' };
    } catch {
      return { providerLogout: 'failed' };
    }
  }

  private setSession(session: OidcSession | undefined): void {
    if (session?.accessToken === this.current?.accessToken) return;
    this.current = session;
    for (const listener of this.listeners) listener(session);
  }
}
