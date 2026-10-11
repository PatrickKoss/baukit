import { useEffect, useState } from 'react';
import { PersistenceIdentityMismatchError } from '@baukit/data-contracts';

import { currentUser, type CurrentUser } from './api';

interface ServerProfileState {
  readonly accessToken?: string;
  readonly subject?: string;
  readonly profile?: CurrentUser;
  readonly error?: Error;
}

export function useServerProfile(
  accessToken: string | undefined,
  subject: string | undefined,
): ServerProfileState {
  const [state, setState] = useState<ServerProfileState>({});
  useEffect(() => {
    if (accessToken === undefined || subject === undefined) return;
    let active = true;
    void currentUser().then(
      (profile) => {
        if (!active) return;
        if (profile.subject !== subject) {
          setState({ accessToken, subject, error: new PersistenceIdentityMismatchError() });
          return;
        }
        setState({ accessToken, subject, profile });
      },
      (cause: unknown) => {
        if (active) setState({
          accessToken,
          subject,
          error: cause instanceof Error ? cause : new Error('Could not confirm the server profile.'),
        });
      },
    );
    return () => { active = false; };
  }, [accessToken, subject]);
  return accessToken === undefined || state.accessToken !== accessToken || state.subject !== subject ? {} : state;
}
