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
  status: {
    unavailable: 'The service is unavailable. Retrying at {time}.',
    crashed: 'Something went wrong',
    crashedDetail: 'The app hit an unexpected error. Try again to reload this screen.',
    retry: 'Try again',
  },
} as const;
