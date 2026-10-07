import { useCallback, useRef } from 'react';
import { Stack } from 'expo-router';
import { useTranslation } from 'react-i18next';
import type { ProfileErasureClient } from '@baukit/api-runtime/erasure';

import { useAppPreferences } from '../src/app-shell';
import { useAuth } from '../src/auth';
import { createDeleteProfileClient, deleteProfile } from '../src/delete-profile';
import { DeleteProfileScreen } from '../src/delete-profile-screen';
import { useAuthenticatedLocalData } from '../src/local-data';

export default function DeleteProfileRoute() {
  const { t } = useTranslation('home');
  const headerOptions = { title: t('erasure.title') };
  const auth = useAuth();
  const localData = useAuthenticatedLocalData();
  const { resetPreferenceIdentity } = useAppPreferences();
  const client = useRef<ProfileErasureClient | null>(null);
  const poll = useCallback((operationId: string, signal: AbortSignal) => {
    if (client.current === null) return Promise.reject(new Error('No deletion request to check.'));
    return client.current.poll(operationId, { signal });
  }, []);
  return (
    <>
      <Stack.Screen options={headerOptions} />
      <DeleteProfileScreen
        available={auth.subject !== undefined && localData.state.status === 'ready'}
        erase={async () => {
          if (auth.subject === undefined) throw new Error('No signed-in account.');
          client.current = await createDeleteProfileClient(auth.subject);
          return deleteProfile({
            subject: auth.subject,
            client: client.current,
            eraseLocalPartition: () => localData.erase(auth.subject ?? ''),
            resetPreferenceIdentity,
          });
        }}
        poll={poll}
      />
    </>
  );
}
