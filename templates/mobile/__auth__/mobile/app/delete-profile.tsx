import { useRef } from 'react';
import type { ProfileErasureClient } from '@baukit/api-runtime/erasure';

import { useAppPreferences } from '../src/app-shell';
import { useOidcAuth } from '../src/auth';
import { createDeleteProfileClient, deleteProfile } from '../src/delete-profile';
import { DeleteProfileScreen } from '../src/delete-profile-screen';
import { useAuthenticatedLocalData } from '../src/local-data';

export default function DeleteProfileRoute() {
  const auth = useOidcAuth();
  const localData = useAuthenticatedLocalData();
  const { resetPreferenceIdentity } = useAppPreferences();
  const client = useRef<ProfileErasureClient | null>(null);
  return (
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
      poll={(operationId, signal) => {
        if (client.current === null)
          return Promise.reject(new Error('No deletion request to check.'));
        return client.current.poll(operationId, { signal });
      }}
    />
  );
}
