import { useSyncExternalStore } from 'react';

import { apiAvailability, type ApiAvailability, type ApiAvailabilityStore } from './api-availability';
import { germanCatalog } from './localization/de';
import { englishCatalog } from './localization/en';

type StatusCopy = typeof englishCatalog.status | typeof germanCatalog.status;

function currentLocale(): string {
  return navigator.language;
}

export function statusCopy(locale: string = currentLocale()): StatusCopy {
  return locale.toLowerCase().split('-')[0] === 'de' ? germanCatalog.status : englishCatalog.status;
}

export function unavailableMessage(retryAt: number, locale: string = currentLocale()): string {
  const time = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(retryAt);
  return statusCopy(locale).unavailable.replace('{time}', time);
}

export function useApiAvailability(store: ApiAvailabilityStore = apiAvailability): ApiAvailability {
  return useSyncExternalStore(store.subscribe, store.retryAt, store.retryAt);
}
