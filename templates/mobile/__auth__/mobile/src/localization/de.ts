import { germanErasureCopy } from '../delete-profile-copy';

export const germanCatalog = {
  navigation: {
    primary: 'Hauptnavigation',
    collapse: 'Navigation einklappen',
    expand: 'Navigation ausklappen',
    close: 'Menü schließen',
    home: 'Startseite',
    today: 'Heute',
    workspace: 'Arbeitsbereich',
    items: 'Einträge',
    privacy: 'Datenschutz',
  },
  bootstrap: {
    loadingItems: 'Einträge werden geladen…',
  },
  home: {
    erasure: germanErasureCopy,
    emptyItems: 'Noch keine Einträge.',
    itemsTitle: 'Einträge',
  },
} as const;
