import { describe, expect, it } from 'vitest';

import type { PreferenceStore } from './store.js';

export type PreferenceStoreFactory<TValues extends object> = (
  initialValues?: TValues,
) => PreferenceStore<TValues> | Promise<PreferenceStore<TValues>>;

export interface PreferenceStoreContractOptions<TValues extends object> {
  readonly initialValues: TValues;
  readonly patch: Partial<TValues>;
  readonly expectedValues: TValues;
}

export function describePreferenceStoreContract<TValues extends object>(
  makeStore: PreferenceStoreFactory<TValues>,
  options: PreferenceStoreContractOptions<TValues>,
): void {
  describe('PreferenceStore contract', () => {
    it('returns undefined when no values have been stored', async () => {
      const store = await makeStore();
      await expect(store.read()).resolves.toBeUndefined();
    });

    it('reads its initial values', async () => {
      const store = await makeStore(options.initialValues);
      await expect(store.read()).resolves.toEqual(options.initialValues);
    });

    it('merges a patch and returns the complete stored values', async () => {
      const store = await makeStore(options.initialValues);
      await expect(store.patch(options.patch)).resolves.toEqual(options.expectedValues);
      await expect(store.read()).resolves.toEqual(options.expectedValues);
    });

    it('returns complete values when patching an empty store', async () => {
      const store = await makeStore();
      await expect(store.patch(options.patch)).resolves.toEqual(options.expectedValues);
      await expect(store.read()).resolves.toEqual(options.expectedValues);
    });

    it('does not mutate the initial value or patch objects', async () => {
      const initialSnapshot = { ...options.initialValues };
      const patchSnapshot = { ...options.patch };
      const store = await makeStore(options.initialValues);
      await store.patch(options.patch);
      expect(options.initialValues).toEqual(initialSnapshot);
      expect(options.patch).toEqual(patchSnapshot);
    });
  });
}
