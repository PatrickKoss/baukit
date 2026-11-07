import { useCallback, useEffect, useState } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { safeAuthErrorMessage, type SessionExpiredReason } from '@baukit/auth-web';
import {
  isPersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from '@baukit/data-contracts';

import { analytics } from './analytics';
import { currentUser, type CurrentUser } from './api';
import { authClient } from './auth';
import { sessionFailure } from './session-failure';
import {
  useAuthenticatedLocalData,
  type AuthenticatedLocalData,
  type LocalDataState,
} from './local-data';

type CachePartition = Extract<LocalDataState, { status: 'ready' }>['partition'];
import type { SignInSession } from './sign-in/port';

export interface AuthSession {
  readonly authenticated: boolean;
  readonly sessionExpiredReason: SessionExpiredReason | undefined;
  readonly authError: string | undefined;
  readonly setAuthError: (message: string | undefined) => void;
  readonly user: CurrentUser | undefined;
  readonly signInSession: SignInSession | undefined;
  /** True after a retryable failure loading the session; the account stays signed in. */
  readonly sessionUnavailable: boolean;
  readonly retrySession: () => void;
  readonly localData: AuthenticatedLocalData;
  /** The open local partition, once it belongs to the confirmed user. */
  readonly partition: CachePartition | undefined;
  readonly signOut: () => Promise<void>;
  readonly markSignedOut: () => void;
}

function errorMessage(cause: unknown, fallback: string): string {
  return cause instanceof Error ? cause.message : fallback;
}

function useSessionEvents(
  markSignedOut: () => void,
  setReason: (reason: SessionExpiredReason | undefined) => void,
  setAuthenticated: (authenticated: boolean) => void,
  setAuthError: (message: string) => void,
): void {
  useEffect(() => {
    const unsubscribe = authClient.subscribeSessionExpired((event) => {
      setReason(event.reason);
      markSignedOut();
    });
    void authClient
      .handleCallback()
      .then((handled) => {
        if (handled) {
          setReason(undefined);
          setAuthenticated(true);
        }
      })
      .catch((cause: unknown) => {
        setAuthError(safeAuthErrorMessage(cause));
      });
    return unsubscribe;
  }, [markSignedOut, setAuthError, setAuthenticated, setReason]);
}

interface SignedInIdentity {
  readonly signInSession: SignInSession | undefined;
  readonly sessionUnavailable: boolean;
  readonly retrySession: () => void;
}

function useSignInSession(
  authenticated: boolean,
  setAuthError: (message: string) => void,
): SignedInIdentity {
  const [signInSession, setSignInSession] = useState<SignInSession>();
  const [sessionUnavailable, setSessionUnavailable] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!authenticated) {
      return;
    }
    let active = true;
    void authClient
      .session()
      .then((nextSession) => {
        if (!active) return;
        setSessionUnavailable(false);
        setSignInSession(nextSession);
      })
      .catch((cause: unknown) => {
        if (!active) return;
        const failure = sessionFailure(cause);
        if (failure.kind === 'retry') {
          setSessionUnavailable(true);
        } else {
          setAuthError(failure.message);
        }
      });
    return () => {
      active = false;
    };
  }, [attempt, authenticated, setAuthError]);
  const retrySession = useCallback((): void => {
    setSessionUnavailable(false);
    setAttempt((current) => current + 1);
  }, []);
  return { signInSession, sessionUnavailable, retrySession };
}

function useCurrentUser(
  authenticated: boolean,
  setUser: (user: CurrentUser) => void,
  setAuthError: (message: string) => void,
): void {
  useEffect(() => {
    if (!authenticated) {
      return;
    }
    let active = true;
    void currentUser()
      .then((nextUser) => {
        if (active) setUser(nextUser);
      })
      .catch((cause: unknown) => {
        if (active) setAuthError(errorMessage(cause, 'Could not confirm account identity.'));
      });
    return () => {
      active = false;
    };
  }, [authenticated, setAuthError, setUser]);
}

/** Identifies the user for analytics once the server confirms the partition's subject. */
function useSyncAdoption(
  partition: CachePartition | undefined,
  user: CurrentUser | undefined,
  blockIdentityMismatch: AuthenticatedLocalData['blockIdentityMismatch'],
  setAuthError: (message: string) => void,
): void {
  useEffect(() => {
    if (partition === undefined || user === undefined) return;
    void recheckServerSubjectBeforeSyncAdoption({
      partitionSubject: partition.subject,
      readServerSubject: () => Promise.resolve(user.id),
      adopt: () => {
        analytics.identify(user.id);
      },
    }).catch((cause: unknown) => {
      if (isPersistenceIdentityMismatchError(cause)) {
        void blockIdentityMismatch(cause).catch(() => {
          setAuthError('Could not safely close local account data.');
        });
      }
    });
  }, [blockIdentityMismatch, partition, setAuthError, user]);
}

/** Sign-in state, the confirmed user and their local data partition. */
export function useAuthSession(queryClient: QueryClient): AuthSession {
  const [authenticated, setAuthenticated] = useState(authClient.hasSession());
  const [sessionExpiredReason, setSessionExpiredReason] = useState<SessionExpiredReason>();
  const [authError, setAuthError] = useState<string>();
  const [user, setUser] = useState<CurrentUser>();
  const localData = useAuthenticatedLocalData(user?.id, sessionExpiredReason, queryClient);
  const markSignedOut = useCallback((): void => {
    setAuthenticated(false);
    setUser(undefined);
  }, []);
  useSessionEvents(markSignedOut, setSessionExpiredReason, setAuthenticated, setAuthError);
  const { signInSession, sessionUnavailable, retrySession } = useSignInSession(
    authenticated,
    setAuthError,
  );
  useCurrentUser(authenticated, setUser, setAuthError);
  const partition =
    localData.state.status === 'ready' &&
    localData.state.partition.subject === user?.id &&
    sessionExpiredReason === undefined
      ? localData.state.partition
      : undefined;
  useSyncAdoption(partition, user, localData.blockIdentityMismatch, setAuthError);
  return {
    authenticated,
    sessionExpiredReason,
    authError,
    setAuthError,
    user,
    signInSession,
    sessionUnavailable,
    retrySession,
    localData,
    partition,
    markSignedOut,
    signOut: async () => {
      setAuthError(undefined);
      try {
        await localData.clear();
        markSignedOut();
        await authClient.signOut();
      } catch (cause) {
        setAuthError(errorMessage(cause, 'Could not close local account data.'));
      }
    },
  };
}
