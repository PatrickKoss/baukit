import { useCallback, useEffect, useState } from 'react';
import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import {
  safeAuthErrorMessage,
  type OidcSession,
  type SignInResult,
  type SignOutResult,
} from '@baukit/auth-native';
import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';

import { signInFeedback } from './auth-feedback';

completeExpoAuthSession();

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['oidcIssuer'];
const configuredClientId: unknown = Constants.expoConfig?.extra?.['oidcClientId'];
const issuer =
  typeof configuredIssuer === 'string'
    ? configuredIssuer
    : 'http://localhost:8081/realms/{{ context.app_name }}';
const clientId =
  typeof configuredClientId === 'string' ? configuredClientId : '{{ context.app_name }}-mobile';
const redirectUri = AuthSession.makeRedirectUri({
  scheme: '{{ context.app_name }}',
  path: 'oauth',
});

const client = createExpoOidcClient({
  issuer,
  clientId,
  redirectUri,
  scopes: ['openid', 'profile', 'email'],
  offlineAccess: true,
  storageKeyPrefix: '{{ context.app_name }}:oidc',
});

export interface OidcAuth {
  readonly accessToken?: string;
  readonly subject?: string;
  readonly ready: boolean;
  readonly error?: string;
  readonly announcement?: string;
  readonly signIn: () => Promise<SignInResult | undefined>;
  readonly signOut: () => Promise<SignOutResult | undefined>;
}

export function useOidcAuth(): OidcAuth {
  const [session, setSession] = useState<OidcSession>();
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [announcement, setAnnouncement] = useState<string>();

  useEffect(() => {
    let active = true;
    const unsubscribe = client.subscribe((nextSession) => {
      if (active) {
        setSession(nextSession);
      }
    });
    void client
      .initialize()
      .then((nextSession) => {
        if (active) {
          setSession(nextSession);
          setReady(true);
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(safeAuthErrorMessage(cause));
          setReady(true);
        }
      });
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (session === undefined) {
      return;
    }
    const delay = Math.max(session.expiresAt - Date.now() - 30_000, 0);
    const timeout = setTimeout(() => {
      void client.accessToken().catch((cause: unknown) => {
        setError(safeAuthErrorMessage(cause));
      });
    }, delay);
    return () => {
      clearTimeout(timeout);
    };
  }, [session]);

  const signIn = useCallback(async (): Promise<SignInResult | undefined> => {
    setError(undefined);
    setAnnouncement(undefined);
    try {
      const result = await client.signIn();
      setAnnouncement(signInFeedback(result));
      return result;
    } catch (cause) {
      setError(safeAuthErrorMessage(cause));
      return undefined;
    }
  }, []);

  const signOut = useCallback(async (): Promise<SignOutResult | undefined> => {
    setError(undefined);
    setAnnouncement(undefined);
    try {
      return await client.signOut();
    } catch (cause) {
      setError(safeAuthErrorMessage(cause));
      return undefined;
    }
  }, []);

  return {
    ...(session?.accessToken === undefined ? {} : { accessToken: session.accessToken }),
    ...(session?.subject === undefined ? {} : { subject: session.subject }),
    ready,
    ...(error === undefined ? {} : { error }),
    ...(announcement === undefined ? {} : { announcement }),
    signIn,
    signOut,
  };
}
