import { isApiUnavailableError } from '@baukit/api-runtime';
import type { FetchImplementation } from '@baukit/api-runtime';

/** Epoch milliseconds when the API cooldown ends, or `undefined` while the API is reachable. */
export type ApiAvailability = number | undefined;

export interface ApiAvailabilityStore {
  readonly retryAt: () => ApiAvailability;
  readonly subscribe: (listener: () => void) => () => void;
  readonly markUnavailable: (retryAt: number) => void;
  readonly markAvailable: () => void;
}

/** Largest delay `setTimeout` accepts; longer ones fire at once. */
const MAX_TIMER_DELAY_MS = 2 ** 31 - 1;

/** Holds the cooldown reported by `ApiUnavailableError` and clears it once `retryAt` passes. */
export function createApiAvailabilityStore(now: () => number = Date.now): ApiAvailabilityStore {
  const listeners = new Set<() => void>();
  let current: ApiAvailability;
  let expiry: ReturnType<typeof setTimeout> | undefined;

  const publish = (next: ApiAvailability): void => {
    if (next === current) return;
    current = next;
    listeners.forEach((listener) => {
      listener();
    });
  };
  const cancelExpiry = (): void => {
    if (expiry === undefined) return;
    clearTimeout(expiry);
    expiry = undefined;
  };

  return {
    retryAt: () => current,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    markUnavailable: (retryAt) => {
      cancelExpiry();
      publish(retryAt);
      expiry = setTimeout(
        () => {
          expiry = undefined;
          publish(undefined);
        },
        Math.min(Math.max(retryAt - now(), 0), MAX_TIMER_DELAY_MS),
      );
    },
    markAvailable: () => {
      cancelExpiry();
      publish(undefined);
    },
  };
}

/** Feeds every outcome of `fetch` into `store`: a cooldown rejection opens it, any response closes it. */
export function trackApiAvailability(
  fetch: FetchImplementation,
  store: ApiAvailabilityStore,
): FetchImplementation {
  return async (input, init) => {
    try {
      const response = await fetch(input, init);
      store.markAvailable();
      return response;
    } catch (cause) {
      if (isApiUnavailableError(cause)) store.markUnavailable(cause.retryAt);
      throw cause;
    }
  };
}

export const apiAvailability = createApiAvailabilityStore();
