import { unverifiedSignInSessionFromJwt } from '@baukit/api-runtime';
import type { ClearSessionOptions, SessionExpiredEvent } from '@baukit/auth-web';

import type { SignInPort } from './port';

/** The calls shared by the OIDC, Clerk and WorkOS browser clients. */
export interface ProviderClient {
  hasSession(): boolean;
  login(): Promise<void>;
  handleCallback(): Promise<boolean>;
  accessToken(options?: { readonly forceRefresh?: boolean }): Promise<string | undefined>;
  subscribeSessionExpired(listener: (event: SessionExpiredEvent) => void): () => void;
  logout(): Promise<boolean>;
  clearSession(options?: ClearSessionOptions): void;
}

const inBrowser = (): boolean => typeof window !== 'undefined';

/** Builds the client on first use in a browser and maps it onto the port. */
export function createSignInPort(providerLabel: string, create: () => ProviderClient): SignInPort {
  let instance: ProviderClient | undefined;
  const client = (): ProviderClient => (instance ??= create());
  const accessToken = (options: { readonly forceRefresh?: boolean } = {}) =>
    inBrowser() ? client().accessToken(options) : Promise.resolve(undefined);
  return {
    providerLabel,
    hasSession: () => inBrowser() && client().hasSession(),
    handleCallback: () => (inBrowser() ? client().handleCallback() : Promise.resolve(false)),
    signIn: () => client().login(),
    signOut: () => (inBrowser() ? client().logout() : Promise.resolve(false)),
    accessToken,
    session: async () => {
      const token = await accessToken();
      return token === undefined ? undefined : unverifiedSignInSessionFromJwt(token);
    },
    subscribeSessionExpired: (listener) =>
      inBrowser() ? client().subscribeSessionExpired(listener) : () => undefined,
    clearSession: (options) => {
      client().clearSession(options);
    },
  };
}
