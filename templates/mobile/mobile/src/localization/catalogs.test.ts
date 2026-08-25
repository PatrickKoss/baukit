import { compareCatalogKeys } from '@baukit/localization-core';

import { germanCatalog } from './de';
import { englishCatalog } from './en';

describe.each(['bootstrap', 'home'] as const)('%s catalog', (namespace) => {
  it('has the same keys in German as in English', () => {
    expect(compareCatalogKeys(englishCatalog[namespace], germanCatalog[namespace])).toEqual({
      missing: [],
      extra: [],
    });
  });
});
