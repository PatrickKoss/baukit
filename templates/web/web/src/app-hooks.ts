import { useEffect, useState } from 'react';
import type { ConsentState } from '@baukit/analytics-core';

import { analytics } from './analytics';
import type { Item } from './api';

export interface Consent {
  readonly consent: ConsentState;
  readonly choose: (next: ConsentState) => void;
}

/** The analytics consent choice, kept in sync with the analytics client. */
export function useConsent(): Consent {
  const [consent, setConsent] = useState<ConsentState>(analytics.consent);
  return {
    consent,
    choose: (next) => {
      analytics.setConsent(next);
      setConsent(next);
    },
  };
}

/** Records `items_viewed` once items load, if the user allowed analytics. */
export function useItemsViewedEvent(items: readonly Item[] | undefined, consent: ConsentState): void {
  useEffect(() => {
    if (items !== undefined && consent === 'granted') {
      analytics.capture({ name: 'items_viewed', properties: { count: items.length } });
    }
  }, [consent, items]);
}

/** `window.location.search`, refreshed when the navigation shell reports a location change. */
export function useLocationSearch(): readonly [string, () => void] {
  const [search, setSearch] = useState(window.location.search);
  return [
    search,
    () => {
      setSearch(window.location.search);
    },
  ];
}
