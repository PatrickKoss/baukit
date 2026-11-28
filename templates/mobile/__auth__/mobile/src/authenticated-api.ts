import { createApiRuntime } from '@baukit/api-runtime';
import type {
  ApiEnvironmentConfig,
  ApiRuntime,
  ApiRuntimeOptions,
  FetchImplementation,
} from '@baukit/api-runtime';

export interface AuthTokenClient {
  clearSession(options?: { readonly reason?: 'profile_erased' }): void | Promise<void>;
  accessToken(options?: { readonly forceRefresh?: boolean }): Promise<string | undefined>;
}

export interface AuthenticatedApiOptions extends ApiEnvironmentConfig {
  readonly auth: AuthTokenClient;
  readonly fetch?: FetchImplementation;
  readonly cooldown?: ApiRuntimeOptions['cooldown'];
  readonly onProfileErased?: () => void | Promise<void>;
  /** Confirmed profile ID; a 401 from an earlier profile's request fires no signal. */
  readonly sessionSubject?: () => string | undefined;
}

/** Composes token refresh with the runtime's explicit one-replay 401 handshake. */
export function createAuthenticatedApiRuntime(options: AuthenticatedApiOptions): ApiRuntime {
  return createApiRuntime({
    baseUrl: options.baseUrl,
    environment: options.environment,
    ...(options.cooldown === undefined ? {} : { cooldown: options.cooldown }),
    ...(options.fetch === undefined ? {} : { fetch: options.fetch }),
    ...(options.sessionSubject === undefined ? {} : { sessionSubject: options.sessionSubject }),
    tokenProvider: async () => (await options.auth.accessToken()) ?? null,
    onProfileErased: async () => {
      await options.auth.clearSession({ reason: 'profile_erased' });
      await options.onProfileErased?.();
    },
    onUnauthorized: async ({ canRetry }) => {
      if (!canRetry) {
        return 'handled';
      }
      const token = await options.auth.accessToken({ forceRefresh: true });
      return token === undefined ? 'handled' : 'retry-once';
    },
  });
}
