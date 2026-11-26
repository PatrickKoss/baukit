import { useSyncExternalStore } from 'react';

import { apiAvailability, type ApiAvailability, type ApiAvailabilityStore } from './api-availability';

export function useApiAvailability(store: ApiAvailabilityStore = apiAvailability): ApiAvailability {
  return useSyncExternalStore(store.subscribe, store.retryAt, store.retryAt);
}

export function formatRetryTime(retryAt: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(retryAt);
}
