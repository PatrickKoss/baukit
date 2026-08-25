import { describe, expect, it } from 'vitest';

import {
  createRepositoryPreferenceStore,
  RepositoryPreferenceStore,
  type PreferenceRecordRepository,
} from './store.js';
import { describePreferenceStoreContract } from './vitest.js';

interface ContractPreferences {
  readonly language: string;
  readonly theme: string;
  readonly gameLayerEnabled: boolean;
}

/** A stored record whose column names deliberately differ from the preference keys. */
interface SettingsRecord {
  readonly subject_id: string;
  readonly language: string;
  readonly theme: string;
  readonly game_layer_enabled: boolean;
  readonly updated_at: string;
}

type SettingsRecordPatch = Partial<Omit<SettingsRecord, 'subject_id' | 'updated_at'>>;

const defaults: ContractPreferences = {
  language: 'system',
  theme: 'dark',
  gameLayerEnabled: false,
};

const SUBJECT = 'subject-1';

class FakeSettingsRepository implements PreferenceRecordRepository<
  string,
  SettingsRecord,
  SettingsRecordPatch
> {
  readonly #records = new Map<string, SettingsRecord>();
  upsertCalls = 0;

  seed(subjectId: string, values: ContractPreferences): void {
    this.#records.set(subjectId, {
      subject_id: subjectId,
      language: values.language,
      theme: values.theme,
      game_layer_enabled: values.gameLayerEnabled,
      updated_at: '2026-01-01T00:00:00.000Z',
    });
  }

  get(subjectId: string): Promise<SettingsRecord | undefined> {
    return Promise.resolve(this.#records.get(subjectId));
  }

  upsert(subjectId: string, patch: SettingsRecordPatch): Promise<SettingsRecord> {
    this.upsertCalls += 1;
    const existing = this.#records.get(subjectId);
    const next: SettingsRecord = {
      subject_id: subjectId,
      language: patch.language ?? existing?.language ?? defaults.language,
      theme: patch.theme ?? existing?.theme ?? defaults.theme,
      game_layer_enabled:
        patch.game_layer_enabled ?? existing?.game_layer_enabled ?? defaults.gameLayerEnabled,
      updated_at: '2026-01-02T00:00:00.000Z',
    };
    this.#records.set(subjectId, next);
    return Promise.resolve(next);
  }
}

function toValues(record: SettingsRecord): ContractPreferences {
  return {
    language: record.language,
    theme: record.theme,
    gameLayerEnabled: record.game_layer_enabled,
  };
}

function toRecordPatch(patch: Partial<ContractPreferences>): SettingsRecordPatch {
  return {
    ...(patch.language === undefined ? {} : { language: patch.language }),
    ...(patch.theme === undefined ? {} : { theme: patch.theme }),
    ...(patch.gameLayerEnabled === undefined ? {} : { game_layer_enabled: patch.gameLayerEnabled }),
  };
}

describePreferenceStoreContract(
  (initialValues?: ContractPreferences) => {
    const repository = new FakeSettingsRepository();
    if (initialValues) {
      repository.seed(SUBJECT, initialValues);
    }
    return createRepositoryPreferenceStore({
      repository,
      subjectId: SUBJECT,
      toValues,
      toRecordPatch,
    });
  },
  {
    initialValues: defaults,
    patch: { language: 'de', gameLayerEnabled: true },
    expectedValues: { language: 'de', theme: 'dark', gameLayerEnabled: true },
  },
);

describe('RepositoryPreferenceStore', () => {
  it('treats a null repository result as a missing record', async () => {
    const store = new RepositoryPreferenceStore({
      repository: {
        get: () => Promise.resolve(null),
        upsert: () => Promise.reject(new Error('unused')),
      },
      subjectId: SUBJECT,
      toValues,
      toRecordPatch,
    });

    await expect(store.read()).resolves.toBeUndefined();
  });

  it('reads only the record belonging to its own subject', async () => {
    const repository = new FakeSettingsRepository();
    repository.seed('other-subject', { ...defaults, language: 'fr' });
    const store = new RepositoryPreferenceStore({
      repository,
      subjectId: SUBJECT,
      toValues,
      toRecordPatch,
    });

    await expect(store.read()).resolves.toBeUndefined();
  });

  it('omits untouched preferences from the record patch', async () => {
    const repository = new FakeSettingsRepository();
    repository.seed(SUBJECT, defaults);
    const seen: SettingsRecordPatch[] = [];
    const store = new RepositoryPreferenceStore({
      repository,
      subjectId: SUBJECT,
      toValues,
      toRecordPatch: (patch) => {
        const recordPatch = toRecordPatch(patch);
        seen.push(recordPatch);
        return recordPatch;
      },
    });

    await store.patch({ theme: 'light' });

    expect(seen).toEqual([{ theme: 'light' }]);
  });

  it('projects the record the repository returns rather than the requested patch', async () => {
    const repository = new FakeSettingsRepository();
    repository.seed(SUBJECT, defaults);
    const store = new RepositoryPreferenceStore({
      repository,
      subjectId: SUBJECT,
      toValues,
      toRecordPatch,
    });

    await expect(store.patch({ language: 'de' })).resolves.toEqual({
      ...defaults,
      language: 'de',
    });
    expect(repository.upsertCalls).toBe(1);
  });
});
