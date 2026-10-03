import { germanErasureCopy } from '../delete-profile-copy';

export const germanCatalog = {
  bootstrap: {
    loadingItems: 'Einträge werden geladen…',
  },
  home: {
    erasure: germanErasureCopy,
    emptyItems: 'Noch keine Einträge.',
    itemsTitle: 'Einträge',
  },
} as const;
