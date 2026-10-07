import { createClient, LoginRequiredError, RefreshError } from '@workos-inc/authkit-js';
import { OidcError, type SessionExpiredEvent } from './index.js';

export interface WorkOsWebConfig {
  readonly clientId: string;
  readonly redirectUri: string;
  readonly apiHostname?: string;
}

/** Adapts AuthKit's browser SDK, including its refresh failure notification. */
export class WorkOsWebClient {
  private sdk: Awaited<ReturnType<typeof createClient>> | undefined;
  private loading: Promise<Awaited<ReturnType<typeof createClient>>> | undefined;
  private cleared = false;
  private readonly listeners = new Set<(event: SessionExpiredEvent) => void>();

  public constructor(private readonly config: WorkOsWebConfig) {}

  private load(): Promise<Awaited<ReturnType<typeof createClient>>> {
    this.loading ??= createClient(this.config.clientId, {
      redirectUri: this.config.redirectUri,
      ...(this.config.apiHostname === undefined ? {} : { apiHostname: this.config.apiHostname }),
      onRefreshFailure: () => {
        this.expire();
      },
    }).then((sdk) => {
      this.sdk = sdk;
      return sdk;
    });
    return this.loading;
  }

  public hasSession(): boolean {
    return !this.cleared && this.sdk?.getUser() != null;
  }

  public async handleCallback(): Promise<boolean> {
    await this.load();
    return this.hasSession();
  }

  public async login(): Promise<void> {
    this.cleared = false;
    await (await this.load()).signIn();
  }

  public async accessToken(
    options: { readonly forceRefresh?: boolean } = {},
  ): Promise<string | undefined> {
    const sdk = await this.load();
    if (!this.hasSession()) return undefined;
    try {
      return await sdk.getAccessToken(options);
    } catch (cause) {
      if (
        cause instanceof LoginRequiredError ||
        (cause instanceof RefreshError && !cause.isTransient)
      ) {
        this.expire();
        return undefined;
      }
      throw new OidcError('refresh_failed', { retryable: true });
    }
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
    await (await this.load()).signOut({ returnTo: this.config.redirectUri, navigate: false });
    return true;
  }

  private expire(): void {
    this.clearSession();
    for (const listener of this.listeners)
      listener({ type: 'session-expired', reason: 'refresh_rejected' });
  }
}
