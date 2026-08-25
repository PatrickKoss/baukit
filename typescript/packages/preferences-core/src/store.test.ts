import { describePreferenceStoreContract } from './vitest.js';
import { InMemoryPreferenceStore } from './store.js';

interface ContractPreferences {
  readonly language: string;
  readonly theme: string;
  readonly gameLayerEnabled: boolean;
}

const defaults: ContractPreferences = {
  language: 'system',
  theme: 'dark',
  gameLayerEnabled: false,
};

describePreferenceStoreContract(
  (initialValues?: ContractPreferences) => new InMemoryPreferenceStore(defaults, initialValues),
  {
    initialValues: defaults,
    patch: { language: 'de', gameLayerEnabled: true },
    expectedValues: {
      language: 'de',
      theme: 'dark',
      gameLayerEnabled: true,
    },
  },
);
