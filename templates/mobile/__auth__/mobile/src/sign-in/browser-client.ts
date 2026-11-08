import type { OidcSession, SessionExpiredEvent } from '@baukit/auth-native';
import { tokenTiming } from '@baukit/auth-native/workos';

import type { SignInClient } from './port';

/** The calls shared by the `@baukit/auth-web` browser clients (OIDC session, Clerk, WorkOS). */
export interface BrowserProviderClient {
  login(): Promise<void>;
  handleCallback(): Promise<boolean>;
  accessToken(options?: { readonly forceRefresh?: boolean }): Promise<string | undefined>;
  subscribeSessionExpired(
    listener: (event: { readonly type: 'session-expired'; readonly reason: string }) => void,
  ): () => void;
  logout(): Promise<boolean>;
  clearSession(options?: { readonly reason?: 'profile_erased' }): void;
}

function expiredEvent(reason: string): SessionExpiredEvent | undefined {
  if (reason === 'refresh_rejected' || reason === 'profile_erased') {
    return { type: 'session-expired', reason };
  }
  return undefined;
}

/**
 * Maps a browser client onto the app's sign-in client for the Expo web build.
 * The browser client keeps the access token in memory and leaves refresh to an
 * HttpOnly cookie or the vendor SDK, so nothing token-shaped reaches web storage.
 */
export function browserSignInClient(create: () => BrowserProviderClient): SignInClient {
  let instance: BrowserProviderClient | undefined;
  const provider = (): BrowserProviderClient => (instance ??= create());
  const listeners = new Set<(session: OidcSession | undefined) => void>();
  let current: OidcSession | undefined;

  const publish = (session: OidcSession | undefined): void => {
    current = session;
    for (const listener of listeners) listener(session);
  };

  const accessToken = async (options: { readonly forceRefresh?: boolean } = {}) => {
    const issuedAt = Date.now();
    const token = await provider().accessToken(options);
    if (token === undefined) {
      if (current !== undefined) publish(undefined);
      return undefined;
    }
    const timing = tokenTiming(token);
    if (current?.accessToken !== token) {
      publish({ subject: timing.subject, accessToken: token, issuedAt, expiresAt: timing.expiresAt });
    }
    return token;
  };

  return {
    initialize: async () => {
      await provider().handleCallback();
      await accessToken();
      return current;
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    subscribeSessionExpired: (listener) =>
      provider().subscribeSessionExpired((event) => {
        publish(undefined);
        const mapped = expiredEvent(event.reason);
        if (mapped !== undefined) listener(mapped);
      }),
    accessToken,
    signIn: async () => {
      await provider().login();
      const token = await accessToken();
      if (token === undefined || current === undefined) {
        return { status: 'cancelled', reason: 'dismiss' };
      }
      return { status: 'success', subject: current.subject };
    },
    signOut: async () => {
      publish(undefined);
      try {
        return { providerLogout: (await provider().logout()) ? 'completed' : 'unavailable' };
      } catch {
        return { providerLogout: 'failed' };
      }
    },
    clearSession: (options = {}) => {
      provider().clearSession(options);
      publish(undefined);
      return Promise.resolve();
    },
  };
}
