import { describe, expect, it } from '@jest/globals';
import { InMemoryRecordStore } from '@baukit/data-contracts';

import {
  AppPreferenceRuntime,
  defaultAppPreferences,
  type AppPreferenceRecord,
  type AppPreferences,
} from './app-preferences';

describe('app preferences', () => {
  it('hydrates and updates a subject record through the generated record store', async () => {
    const records = new InMemoryRecordStore<AppPreferenceRecord>();
    await records.put({
      id: 'subject-a',
      language: 'de',
      theme: 'dark',
      analytics_consent: 'denied',
    });
    const visible: AppPreferences[] = [];
    const runtime = new AppPreferenceRuntime(records, (values) =>
      visible.push(values),
    );

    await expect(runtime.switchIdentity('subject-a')).resolves.toEqual({
      language: 'de',
      theme: 'dark',
      analyticsConsent: 'denied',
    });
    await expect(
      runtime.update({ language: 'system', analyticsConsent: 'granted' }),
    ).resolves.toEqual({
      language: 'system',
      theme: 'dark',
      analyticsConsent: 'granted',
    });
    await expect(records.get('subject-a')).resolves.toEqual({
      id: 'subject-a',
      language: 'system',
      theme: 'dark',
      analytics_consent: 'granted',
    });
    expect(visible.at(-1)?.analyticsConsent).toBe('granted');
  });

  it('publishes defaults before reading another identity and resets while signed out', async () => {
    const records = new InMemoryRecordStore<AppPreferenceRecord>();
    await records.put({
      id: 'subject-a',
      language: 'de',
      theme: 'dark',
      analytics_consent: 'granted',
    });
    await records.put({
      id: 'subject-b',
      language: 'system',
      theme: 'light',
      analytics_consent: 'denied',
    });
    const visible: AppPreferences[] = [];
    const runtime = new AppPreferenceRuntime(records, (values) =>
      visible.push(values),
    );
    await runtime.switchIdentity('subject-a');

    const switched = runtime.switchIdentity('subject-b');
    expect(visible.at(-1)).toEqual(defaultAppPreferences);
    await expect(switched).resolves.toEqual({
      language: 'system',
      theme: 'light',
      analyticsConsent: 'denied',
    });

    const reset = runtime.switchIdentity(null);
    expect(visible.at(-1)).toEqual(defaultAppPreferences);
    await expect(reset).resolves.toEqual(defaultAppPreferences);
  });

  it('publishes an optimistic theme and rolls it back when persistence fails', async () => {
    let writesFail = false;
    const records = new InMemoryRecordStore<AppPreferenceRecord>(
      undefined,
      () => {
        if (writesFail) {
          throw new Error('theme persistence failed');
        }
      },
    );
    await records.put({
      id: 'subject-a',
      language: 'system',
      theme: 'system',
      analytics_consent: 'unknown',
    });
    const visible: AppPreferences[] = [];
    const runtime = new AppPreferenceRuntime(records, (values) =>
      visible.push(values),
    );
    await runtime.switchIdentity('subject-a');
    writesFail = true;

    const update = runtime.update({ theme: 'dark' });
    expect(visible.at(-1)?.theme).toBe('dark');
    await expect(update).rejects.toThrow('theme persistence failed');
    expect(visible.at(-1)?.theme).toBe('system');
    expect(runtime.preferences.theme).toBe('system');
  });
});
