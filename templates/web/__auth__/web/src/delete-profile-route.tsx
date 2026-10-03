import { useCallback, useRef } from 'react';
import type { ProfileErasureClient } from '@baukit/api-runtime/erasure';

import { createDeleteProfileClient, deleteProfile } from './delete-profile';
import { DeleteProfileScreen } from './delete-profile-screen';

export function DeleteProfileRoute({
  subject,
  eraseLocalPartition,
  onSignedOut,
  ready,
}: {
  readonly subject: string | undefined;
  readonly eraseLocalPartition: () => Promise<void>;
  readonly onSignedOut: () => void;
  readonly ready: boolean;
}) {
  const client = useRef<ProfileErasureClient | null>(null);
  const poll = useCallback((operationId: string, signal: AbortSignal) => {
    if (client.current === null) return Promise.reject(new Error('No deletion request to check.'));
    return client.current.poll(operationId, { signal });
  }, []);
  return (
    <DeleteProfileScreen
      available={subject !== undefined && ready}
      erase={async () => {
        if (subject === undefined) throw new Error('No signed-in account.');
        client.current = await createDeleteProfileClient(subject);
        return deleteProfile({
          client: client.current,
          eraseLocalPartition,
          onSignedOut,
        });
      }}
      poll={poll}
    />
  );
}
