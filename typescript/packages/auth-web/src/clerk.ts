import { Clerk } from '@clerk/clerk-js';
import type { SessionExpiredEvent } from './index.js';

export interface ClerkWebConfig {
  readonly publishableKey: string;
  readonly redirectUri: string;
  readonly jwtTemplate?: string;
}

/** Adapts Clerk's browser SDK to the product authentication contract. */
export class ClerkWebClient {
  private readonly sdk: Clerk;
  private loaded: Promise<void> | undefined;
  private cleared = false;
  private readonly listeners = new Set<(event: SessionExpiredEvent) => void>();

  public constructor(private readonly config: ClerkWebConfig) {
    this.sdk = new Clerk(config.publishableKey);
  }

  private load(): Promise<void> {
    this.loaded ??= this.sdk.load().then(() => undefined);
    return this.loaded;
  }

  public hasSession(): boolean {
    return !this.cleared && this.sdk.session != null;
  }

  public async handleCallback(): Promise<boolean> {
    await this.load();
    return this.hasSession();
  }

  public async login(): Promise<void> {
    await this.load();
    this.cleared = false;
    await this.sdk.redirectToSignIn({ signInForceRedirectUrl: this.config.redirectUri });
  }

  public async accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    await this.load();
    if (!this.hasSession()) return undefined;
    const token = await this.sdk.session?.getToken({
      skipCache: options.forceRefresh ?? false,
      ...(this.config.jwtTemplate === undefined ? {} : { template: this.config.jwtTemplate }),
    });
    if (token == null) {
      this.clearSession();
      for (const listener of this.listeners)
        listener({ type: 'session-expired', reason: 'refresh_rejected' });
      return undefined;
    }
    return token;
  }

  public subscribeSessionExpired(listener: (event: SessionExpiredEvent) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  public clearSession(): void {
    this.cleared = true;
  }

  public async logout(): Promise<boolean> {
    this.clearSession();
    await this.load();
    await this.sdk.signOut({ redirectUrl: this.config.redirectUri });
    return true;
  }
}
