import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import {
  assertFeatureKeysMatch,
  catalogFeatureKeys,
  featureKeyDrift,
  PlanCatalogError,
} from './index.js';

const catalog: unknown = JSON.parse(
  readFileSync(new URL('../../../../fixtures/plans/v1/catalog.json', import.meta.url), 'utf8'),
);
const declared = [
  'eigenruhe.custom_programs',
  'eigenruhe.insights_extended',
  'eigenruhe.nachtruhe',
  'eigenruhe.studio',
];

describe('plan catalog feature keys', () => {
  it('reads the keys the recorded hub catalog grants a product', () => {
    expect(catalogFeatureKeys(catalog, 'eigenruhe')).toEqual(declared);
    expect(() => {
      assertFeatureKeysMatch(catalog, 'eigenruhe', declared);
    }).not.toThrow();
  });

  it('reports keys on either side that the other lacks', () => {
    expect(
      featureKeyDrift(catalog, 'eigenruhe', [
        'eigenruhe.custom_programs',
        'eigenruhe.offline_mode',
      ]),
    ).toEqual({
      notInCatalog: ['eigenruhe.offline_mode'],
      notDeclared: ['eigenruhe.insights_extended', 'eigenruhe.nachtruhe', 'eigenruhe.studio'],
    });
    expect(() => {
      assertFeatureKeysMatch(catalog, 'eigenruhe', ['eigenruhe.offline_mode']);
    }).toThrow(/not in catalog \["eigenruhe.offline_mode"\]/);
  });

  it('rejects an unknown product and a malformed catalog', () => {
    expect(() => catalogFeatureKeys(catalog, 'unknown_product')).toThrow(PlanCatalogError);
    for (const broken of [
      null,
      {},
      { plans: [{}] },
      { plans: [{ includedProducts: [{ productId: 'eigenruhe', features: [1] }] }] },
    ]) {
      expect(() => catalogFeatureKeys(broken, 'eigenruhe')).toThrow(PlanCatalogError);
    }
  });
});
