import { englishErasureCopy } from '../delete-profile-copy';

export const englishCatalog = {
  bootstrap: {
    loadingItems: 'Loading items…',
  },
  home: {
    erasure: englishErasureCopy,
    emptyItems: 'No items yet.',
    itemsTitle: 'Items',
  },
} as const;
