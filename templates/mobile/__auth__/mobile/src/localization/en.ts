import { englishErasureCopy } from '../delete-profile-copy';

export const englishCatalog = {
  navigation: {
    primary: 'Primary',
    collapse: 'Collapse navigation',
    expand: 'Expand navigation',
    close: 'Close menu',
    home: 'Home',
    today: 'Today',
    workspace: 'Workspace',
    items: 'Items',
    privacy: 'Privacy',
  },
  bootstrap: {
    loadingItems: 'Loading items…',
  },
  home: {
    erasure: englishErasureCopy,
    emptyItems: 'No items yet.',
    itemsTitle: 'Items',
  },
} as const;
