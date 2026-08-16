import { createApiRuntime } from '@baukit/api-runtime';
import type { ApiEnvironmentConfig, ApiRuntime, FetchImplementation } from '@baukit/api-runtime';

export interface AuthTokenClient {
  accessToken(options?: { readonly forceRefresh?: boolean }): Promise<string | undefined>;
}

export interface AuthenticatedApiOptions extends ApiEnvironmentConfig {
  readonly auth: AuthTokenClient;
  readonly fetch?: FetchImplementation;
}

/** Composes token refresh with the runtime's explicit one-replay 401 handshake. */
export function createAuthenticatedApiRuntime(options: AuthenticatedApiOptions): ApiRuntime {
  return createApiRuntime({
    baseUrl: options.baseUrl,
    environment: options.environment,
    ...(options.fetch === undefined ? {} : { fetch: options.fetch }),
    tokenProvider: async () => (await options.auth.accessToken()) ?? null,
    onUnauthorized: async ({ canRetry }) => {
      if (!canRetry) {
        return 'handled';
      }
      const token = await options.auth.accessToken({ forceRefresh: true });
      return token === undefined ? 'handled' : 'retry-once';
    },
  });
}
