import { QueryClient } from '@tanstack/react-query';

/**
 * `@baukit/api-runtime` already retries with jitter and fails fast during a cooldown, so React Query
 * must not add a second retry layer. Focus refetch is off: many tabs regaining focus after an outage
 * would all fire at once, and the runtime cannot jitter requests it has not seen yet.
 */
export function createAppQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}
