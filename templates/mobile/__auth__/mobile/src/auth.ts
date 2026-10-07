import {
  createContext,
  createElement,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react';
import Constants from 'expo-constants';
{% if not context.auth_clerk %}import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
{% endif %}
import {
{% if not context.auth_clerk %}  appearanceStateDecoration,{% endif %}
  safeAuthErrorMessage,
  type OidcSession,
  type SignInResult,
  type SignOutResult,
} from '@baukit/auth-native';
{% if context.auth_oidc %}import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';
{% elif context.auth_workos %}import { completeExpoAuthSession, createExpoOidcEnvironment } from '@baukit/auth-native/expo';
import { createWorkOsNativeClient } from '@baukit/auth-native/workos';
{% else %}import { createClerkExpoClient } from '@baukit/auth-native/clerk-expo';
{% endif %}

import type { ThemePreference } from './app-preferences';
import { signInFeedback } from './auth-feedback';
import { authStorage } from './auth-storage';

{% if not context.auth_clerk %}import { PRODUCT_NAME } from './product';{% endif %}

{% if not context.auth_clerk %}completeExpoAuthSession();

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['oidcIssuer'];
const configuredClientId: unknown = Constants.expoConfig?.extra?.['oidcClientId'];
{% if context.auth_oidc %}const configuredAudience: unknown = Constants.expoConfig?.extra?.['oidcAudience'];
const configuredResource: unknown = Constants.expoConfig?.extra?.['oidcResource'];
const configuredScopes: unknown = Constants.expoConfig?.extra?.['oidcScopes'];
const offlineAccess = Constants.expoConfig?.extra?.['oidcOfflineAccess'] !== false;
{% endif %}
const issuer =
  typeof configuredIssuer === 'string'
    ? configuredIssuer
    : {% if context.auth_oidc %}`http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`{% else %}'https://api.workos.com/'{% endif %};
const clientId =
  typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-mobile`;
const redirectUri = AuthSession.makeRedirectUri({
  scheme: PRODUCT_NAME,
  path: 'oauth',
});

export const authClient = {% if context.auth_oidc %}createExpoOidcClient{% else %}createWorkOsNativeClient{% endif %}(
  {
    issuer,
    clientId,
    redirectUri,
{% if context.auth_oidc %}    scopes: typeof configuredScopes === 'string' && configuredScopes.trim()
      ? configuredScopes.trim().split(/\s+/u) : ['openid', 'profile', 'email'],
    ...(typeof configuredAudience === 'string' && configuredAudience.trim() ? { audience: configuredAudience } : {}),
    ...(typeof configuredResource === 'string' && configuredResource.trim() ? { resource: configuredResource } : {}),
    offlineAccess,
{% else %}    scopes: ['openid', 'profile', 'email'],
    offlineAccess: true,
{% endif %}
    storageKeyPrefix: `${PRODUCT_NAME}.oidc`,
  },
{% if context.auth_workos %}  createExpoOidcEnvironment({
    randomBytes: (size) => Crypto.getRandomBytesAsync(size),
    storage: authStorage,
  }),
{% else %}  {
    randomBytes: (size) => Crypto.getRandomBytesAsync(size),
    storage: authStorage,
  },
{% endif %}
);

{% else %}const configuredKey: unknown = Constants.expoConfig?.extra?.['clerkPublishableKey'];
const clerk = createClerkExpoClient(typeof configuredKey === 'string' ? configuredKey : '', authStorage);
export const authClient = clerk.client;
{% endif %}
export interface ProductAuth {
  readonly accessToken?: string;
  readonly subject?: string;
  readonly ready: boolean;
  readonly error?: string;
  readonly announcement?: string;
  readonly sessionExpired: boolean;
  /** Passes the app's theme to the login page so it opens in the same mode. */
  readonly signIn: (appearance?: ThemePreference) => Promise<SignInResult | undefined>;
  readonly signOut: () => Promise<SignOutResult | undefined>;
}

const AuthContext = createContext<ProductAuth | undefined>(undefined);

export function AuthProvider({ children }: PropsWithChildren) {
  const auth = useAuthState();
  const content = createElement(AuthContext.Provider, { value: auth }, children);
  return {% if context.auth_clerk %}createElement(clerk.Provider, {}, content){% else %}content{% endif %};
}

export function useAuth(): ProductAuth {
  const auth = useContext(AuthContext);
  if (auth === undefined) {
    throw new Error('useAuth must be used within AuthProvider.');
  }
  return auth;
}

function useAuthState(): ProductAuth {
  const [session, setSession] = useState<OidcSession>();
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [announcement, setAnnouncement] = useState<string>();
  const [sessionExpired, setSessionExpired] = useState(false);

  useEffect(() => {
    let active = true;
    const unsubscribe = authClient.subscribe((nextSession) => {
      if (active) {
        setSession(nextSession);
      }
    });
    const unsubscribeExpired = authClient.subscribeSessionExpired(() => {
      if (active) {
        setSessionExpired(true);
        setAnnouncement('Your session expired. Sign in again to continue.');
      }
    });
    void authClient
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
      unsubscribeExpired();
    };
  }, []);

  useEffect(() => {
    if (session === undefined) {
      return;
    }
    const delay = Math.max(session.expiresAt - Date.now() - 30_000, 0);
    const timeout = setTimeout(() => {
      void authClient.accessToken().catch((cause: unknown) => {
        setError(safeAuthErrorMessage(cause));
      });
    }, delay);
    return () => {
      clearTimeout(timeout);
    };
  }, [session]);

  const signIn = useCallback(
    async ({% if not context.auth_clerk %}appearance?: ThemePreference{% endif %}): Promise<SignInResult | undefined> => {
      setError(undefined);
      setAnnouncement(undefined);
      setSessionExpired(false);
      try {
        const result = await authClient.signIn({% if not context.auth_clerk %}
          appearance === undefined
            ? {}
            : { stateDecoration: appearanceStateDecoration({ mode: appearance }) },
{% endif %});
        setAnnouncement(signInFeedback(result));
        return result;
      } catch (cause) {
        setError(safeAuthErrorMessage(cause));
        return undefined;
      }
    },
    [],
  );

  const signOut = useCallback(async (): Promise<SignOutResult | undefined> => {
    setError(undefined);
    setAnnouncement(undefined);
    try {
      return await authClient.signOut();
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
    sessionExpired,
    signIn,
    signOut,
  };
}
