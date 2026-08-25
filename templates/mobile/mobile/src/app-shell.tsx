import {
  createContext,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';
import { ActivityIndicator, StyleSheet, View } from 'react-native';
import * as SQLite from 'expo-sqlite';
import type { AnalyticsClient, ConsentState } from '@baukit/analytics-core';
import { InMemoryRecordStore, type RecordStore } from '@baukit/data-contracts';

import { loadAnalytics } from './analytics';
import type { ProductEvent } from './analytics-client';
import {
  AppPreferenceRuntime,
  defaultAppPreferences,
  type AppPreferenceRecord,
  type AppPreferences,
} from './app-preferences';
import { initializeI18n } from './localization/i18n';
import { createAppPreferenceRecordStore } from './record-store';
import { theme } from './theme';

const DEVICE_PREFERENCE_SUBJECT = 'device';

export interface AnalyticsConsent {
  readonly analytics: AnalyticsClient<ProductEvent> | undefined;
  readonly consent: ConsentState;
  readonly setConsent: (consent: ConsentState) => Promise<void>;
}

export interface AppPreferencesContextValue extends AnalyticsConsent {
  readonly preferences: AppPreferences;
  readonly updatePreferences: (patch: Partial<AppPreferences>) => Promise<AppPreferences>;
  readonly resetPreferenceIdentity: () => Promise<void>;
}

interface AppShellProps extends PropsWithChildren {
  readonly preferenceSubjectId?: string | null;
}

const AppPreferencesContext = createContext<AppPreferencesContextValue | undefined>(undefined);

export function AppShell({ children, preferenceSubjectId }: AppShellProps) {
  const subjectId =
    preferenceSubjectId === undefined ? DEVICE_PREFERENCE_SUBJECT : preferenceSubjectId;
  const [recordStore, setRecordStore] = useState<RecordStore<AppPreferenceRecord>>();
  const [analytics, setAnalytics] = useState<AnalyticsClient<ProductEvent>>();
  const [preferences, setPreferences] = useState(defaultAppPreferences);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let active = true;
    let database: SQLite.SQLiteDatabase | undefined;
    const openRecordStore = SQLite.openDatabaseAsync('{{ context.app_name }}-preferences.db')
      .then(async (opened) => {
        database = opened;
        return createAppPreferenceRecordStore(opened);
      })
      .catch(async () => {
        await database?.closeAsync().catch(() => undefined);
        database = undefined;
        return new InMemoryRecordStore<AppPreferenceRecord>();
      });
    void Promise.all([openRecordStore, loadAnalytics()]).then(([store, client]) => {
      if (active) {
        setRecordStore(store);
        setAnalytics(client);
      } else {
        void database?.closeAsync();
      }
    });
    return () => {
      active = false;
      void database?.closeAsync();
    };
  }, []);

  const runtime = useMemo(
    () =>
      recordStore === undefined ? undefined : new AppPreferenceRuntime(recordStore, setPreferences),
    [recordStore],
  );

  useEffect(
    () => () => {
      runtime?.stop();
    },
    [runtime],
  );

  const applyPreferences = useCallback(
    async (values: AppPreferences): Promise<void> => {
      analytics?.setConsent(values.analyticsConsent);
      await initializeI18n(values.language);
    },
    [analytics],
  );

  useEffect(() => {
    if (runtime === undefined || analytics === undefined) {
      return;
    }
    let active = true;
    const switched = runtime.switchIdentity(subjectId);
    void applyPreferences(runtime.preferences);
    void switched
      .then(applyPreferences)
      .catch(() => applyPreferences(defaultAppPreferences))
      .finally(() => {
        if (active) {
          setReady(true);
        }
      });
    return () => {
      active = false;
    };
  }, [analytics, applyPreferences, runtime, subjectId]);

  const updatePreferences = useCallback(
    async (patch: Partial<AppPreferences>): Promise<AppPreferences> => {
      if (runtime === undefined) {
        throw new Error('App preferences are not ready.');
      }
      const values = await runtime.update(patch);
      await applyPreferences(values);
      return values;
    },
    [applyPreferences, runtime],
  );

  const resetPreferenceIdentity = useCallback(async (): Promise<void> => {
    if (runtime === undefined) {
      return;
    }
    const reset = runtime.switchIdentity(null);
    await applyPreferences(runtime.preferences);
    await applyPreferences(await reset);
  }, [applyPreferences, runtime]);

  const setConsent = useCallback(
    async (nextConsent: ConsentState): Promise<void> => {
      await updatePreferences({ analyticsConsent: nextConsent });
    },
    [updatePreferences],
  );

  const contextValue = useMemo<AppPreferencesContextValue>(
    () => ({
      analytics,
      consent: preferences.analyticsConsent,
      preferences,
      resetPreferenceIdentity,
      setConsent,
      updatePreferences,
    }),
    [analytics, preferences, resetPreferenceIdentity, setConsent, updatePreferences],
  );

  if (!ready) {
    return (
      <View style={styles.bootstrap}>
        <ActivityIndicator color={theme.color.accent} />
      </View>
    );
  }

  return (
    <AppPreferencesContext.Provider value={contextValue}>{children}</AppPreferencesContext.Provider>
  );
}

export function useAppPreferences(): AppPreferencesContextValue {
  const value = useContext(AppPreferencesContext);
  if (value === undefined) {
    throw new Error('useAppPreferences must be used within AppShell.');
  }
  return value;
}

export function useAnalyticsConsent(): AnalyticsConsent {
  return useAppPreferences();
}

const styles = StyleSheet.create({
  bootstrap: {
    alignItems: 'center',
    justifyContent: 'center',
    flex: 1,
    backgroundColor: theme.color.background,
  },
});
