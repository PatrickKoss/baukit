import type { ConsentState } from '@baukit/analytics-core';
import type { RecordStore } from '@baukit/data-contracts';
import { normalizeLocalePreference } from '@baukit/localization-core';
import {
  createPreferenceController,
  createRepositoryPreferenceStore,
  definePreferenceRegistry,
  InMemoryPreferenceStore,
  type PreferenceController,
  type PreferenceRecordRepository,
} from '@baukit/preferences-core';

import { supportedLocales, type LocalePreference } from './localization/i18n';

export type ThemePreference = 'system' | 'light' | 'dark';

export interface AppPreferences {
  readonly language: LocalePreference;
  readonly theme: ThemePreference;
  readonly analyticsConsent: ConsentState;
}

export interface AppPreferenceRecord {
  readonly id: string;
  readonly language: LocalePreference;
  readonly theme: ThemePreference;
  readonly analytics_consent: ConsentState;
}

type AppPreferenceRecordPatch = Partial<Omit<AppPreferenceRecord, 'id'>>;

export const defaultAppPreferences: AppPreferences = {
  language: 'system',
  theme: 'system',
  analyticsConsent: 'unknown',
};

export const appPreferenceDefinitions = definePreferenceRegistry<AppPreferences>({
  language: {
    key: 'language',
    defaultValue: defaultAppPreferences.language,
    normalize: (value) =>
      normalizeLocalePreference({
        value,
        supported: supportedLocales,
        fallback: 'en',
      }),
    scope: 'identity',
  },
  theme: {
    key: 'theme',
    defaultValue: defaultAppPreferences.theme,
    normalize: (value) =>
      value === 'light' || value === 'dark' || value === 'system' ? value : 'system',
    scope: 'identity',
  },
  analyticsConsent: {
    key: 'analyticsConsent',
    defaultValue: defaultAppPreferences.analyticsConsent,
    normalize: (value) => (value === 'granted' || value === 'denied' ? value : 'unknown'),
    scope: 'identity',
  },
});

class RecordPreferenceRepository implements PreferenceRecordRepository<
  string,
  AppPreferenceRecord,
  AppPreferenceRecordPatch
> {
  public constructor(private readonly records: RecordStore<AppPreferenceRecord>) {}

  public get(subjectId: string): Promise<AppPreferenceRecord | undefined> {
    return this.records.get(subjectId);
  }

  public async upsert(
    subjectId: string,
    patch: AppPreferenceRecordPatch,
  ): Promise<AppPreferenceRecord> {
    const current = await this.records.get(subjectId);
    const next: AppPreferenceRecord = {
      id: subjectId,
      language: defaultAppPreferences.language,
      theme: defaultAppPreferences.theme,
      analytics_consent: defaultAppPreferences.analyticsConsent,
      ...current,
      ...patch,
    };
    await this.records.put(next);
    return next;
  }
}

export class AppPreferenceRuntime {
  readonly #repository: RecordPreferenceRepository;
  readonly #controller: PreferenceController<AppPreferences>;

  public constructor(
    records: RecordStore<AppPreferenceRecord>,
    onVisibleChange: (values: AppPreferences) => void,
  ) {
    this.#repository = new RecordPreferenceRepository(records);
    this.#controller = createPreferenceController({
      definitions: appPreferenceDefinitions,
      store: new InMemoryPreferenceStore(defaultAppPreferences),
      onVisibleChange,
    });
  }

  public get preferences(): AppPreferences {
    return this.#controller.getState().values;
  }

  public switchIdentity(subjectId: string | null): Promise<AppPreferences> {
    const store =
      subjectId === null
        ? new InMemoryPreferenceStore(defaultAppPreferences)
        : createRepositoryPreferenceStore({
            repository: this.#repository,
            subjectId,
            toValues: (record) => ({
              language: record.language,
              theme: record.theme,
              analyticsConsent: record.analytics_consent,
            }),
            toRecordPatch: (patch) => ({
              ...(patch.language === undefined ? {} : { language: patch.language }),
              ...(patch.theme === undefined ? {} : { theme: patch.theme }),
              ...(patch.analyticsConsent === undefined
                ? {}
                : { analytics_consent: patch.analyticsConsent }),
            }),
          });
    return this.#controller.switchIdentity(store);
  }

  public update(patch: Partial<AppPreferences>): Promise<AppPreferences> {
    return this.#controller.update(patch);
  }

  public stop(): void {
    this.#controller.stop();
  }
}
