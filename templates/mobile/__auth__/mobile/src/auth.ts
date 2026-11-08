import {
  createContext,
  createElement,
  type Dispatch,
  type PropsWithChildren,
  type ReactElement,
  type SetStateAction,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react';
{% if context.auth_oidc %}import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
{% endif %}import { unverifiedSignInSessionFromJwt } from '@baukit/api-runtime';
import {
  appearanceStateDecoration,
  safeAuthErrorMessage,
  type OidcSession,
  type SessionExpiredReason,
  type SignInResult,
  type SignOutResult,
} from '@baukit/auth-native';
{% if context.auth_oidc %}import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';
{% endif %}{% if context.auth_oidc and context.pwa and not context.web %}import { BrowserSessionClient } from '@baukit/auth-web';
import { Platform } from 'react-native';
{% endif %}
import type { ThemePreference } from './app-preferences';
import { signInFeedback } from './auth-feedback';
{% if context.auth_oidc and context.pwa and not context.web %}import { apiBaseUrl } from './api-base-url';
{% endif %}{% if context.auth_oidc %}import { authStorage } from './auth-storage';
{% endif %}{% if context.auth_oidc and context.pwa and not context.web %}import { browserSignInClient } from './sign-in/browser-client';
{% endif %}import type { {% if context.auth_oidc %}SignInAdapter, SignInClient, {% endif %}SignInSession } from './sign-in/port';
{% if context.auth_oidc %}
import { PRODUCT_NAME } from './product';

completeExpoAuthSession();

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['oidcIssuer'];
const configuredClientId: unknown = Constants.expoConfig?.extra?.['oidcClientId'];
const configuredAudience: unknown = Constants.expoConfig?.extra?.['oidcAudience'];
const configuredResource: unknown = Constants.expoConfig?.extra?.['oidcResource'];
const configuredScopes: unknown = Constants.expoConfig?.extra?.['oidcScopes'];
const offlineAccess = Constants.expoConfig?.extra?.['oidcOfflineAccess'] !== false;

const scopes =
  typeof configuredScopes === 'string' && configuredScopes.trim()
    ? configuredScopes.trim().split(/\s+/u)
    : ['openid', 'profile', 'email'];
const issuer =
  typeof configuredIssuer === 'string'
    ? configuredIssuer
    : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`;
const tokenBinding = {
  ...(typeof configuredAudience === 'string' && configuredAudience.trim()
    ? { audience: configuredAudience }
    : {}),
  ...(typeof configuredResource === 'string' && configuredResource.trim()
    ? { resource: configuredResource }
    : {}),
};

function nativeClient(): SignInClient {
  return createExpoOidcClient(
    {
      issuer,
      clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-mobile`,
      redirectUri: AuthSession.makeRedirectUri({ scheme: PRODUCT_NAME, path: 'oauth' }),
      scopes,
      ...tokenBinding,
      offlineAccess,
      storageKeyPrefix: `${PRODUCT_NAME}.oidc`,
    },
    {
      randomBytes: (size) => Crypto.getRandomBytesAsync(size),
      storage: authStorage,
    },
  );
}
{% if context.pwa and not context.web %}
const configuredWebClientId: unknown = Constants.expoConfig?.extra?.['oidcWebClientId'];

/** The web build redeems the code through the backend's `/auth/session`, which keeps the refresh token (ADR 0006). */
function browserClient(): SignInClient {
  return browserSignInClient(
    () =>
      new BrowserSessionClient({
        apiBaseUrl: apiBaseUrl || window.location.origin,
        issuer,
        clientId: typeof configuredWebClientId === 'string' ? configuredWebClientId : `${PRODUCT_NAME}-web`,
        redirectUri: `${window.location.origin}/`,
        scopes,
        ...tokenBinding,
        offlineAccess,
        storageKeyPrefix: `${PRODUCT_NAME}:oidc`,
      }),
  );
}
{% endif %}
const signInAdapter: SignInAdapter = {
  providerLabel: 'local Keycloak',
  client: {% if context.pwa and not context.web %}Platform.OS === 'web' ? browserClient() : {% endif %}nativeClient(),
};
{% else %}// Metro swaps this import for ./sign-in/production in production builds (metro.config.js).
import { signInAdapter } from './sign-in/local';
{% endif %}
export const authClient = signInAdapter.client;

export interface ProductAuth {
  readonly accessToken?: string;
  readonly subject?: string;
  readonly ready: boolean;
  readonly error?: string;
  readonly announcement?: string;
  readonly sessionExpired: boolean;
  readonly sessionExpiredReason?: SessionExpiredReason;
  readonly providerLabel: string;
  /** User and organization read from the access token's claims. */
  readonly session?: SignInSession;
  /** Passes the app's theme to the login page so it opens in the same mode. */
  readonly signIn: (appearance?: ThemePreference) => Promise<SignInResult | undefined>;
  readonly signOut: () => Promise<SignOutResult | undefined>;
}

const AuthContext = createContext<ProductAuth | undefined>(undefined);

export function AuthProvider({ children }: PropsWithChildren): ReactElement {
  const auth = useAuthState();
  const content = createElement(AuthContext.Provider, { value: auth }, children);
  return signInAdapter.Provider === undefined
    ? content
    : createElement(signInAdapter.Provider, {}, content);
}

export function useAuth(): ProductAuth {
  const auth = useContext(AuthContext);
  if (auth === undefined) {
    throw new Error('useAuth must be used within AuthProvider.');
  }
  return auth;
}

interface ClientSessionState {
  readonly session: OidcSession | undefined;
  readonly ready: boolean;
  readonly error: string | undefined;
  readonly announcement: string | undefined;
  readonly sessionExpiredReason: SessionExpiredReason | undefined;
  readonly setError: Dispatch<SetStateAction<string | undefined>>;
  readonly setAnnouncement: Dispatch<SetStateAction<string | undefined>>;
  readonly setSessionExpiredReason: Dispatch<SetStateAction<SessionExpiredReason | undefined>>;
}

function useClientSession(): ClientSessionState {
  const [session, setSession] = useState<OidcSession>();
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [announcement, setAnnouncement] = useState<string>();
  const [sessionExpiredReason, setSessionExpiredReason] = useState<SessionExpiredReason>();

  useEffect(() => {
    let active = true;
    const unsubscribe = authClient.subscribe((nextSession) => {
      if (active) {
        setSession(nextSession);
      }
    });
    const unsubscribeExpired = authClient.subscribeSessionExpired((event) => {
      if (active) {
        setSessionExpiredReason(event.reason);
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

  return {
    session,
    ready,
    error,
    announcement,
    sessionExpiredReason,
    setError,
    setAnnouncement,
    setSessionExpiredReason,
  };
}

function useTokenRenewal(
  session: OidcSession | undefined,
  setError: Dispatch<SetStateAction<string | undefined>>,
): void {
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
  }, [session, setError]);
}

function useAuthState(): ProductAuth {
  const {
    session,
    ready,
    error,
    announcement,
    sessionExpiredReason,
    setError,
    setAnnouncement,
    setSessionExpiredReason,
  } = useClientSession();
  useTokenRenewal(session, setError);

  const signIn = useCallback(
    async (appearance?: ThemePreference): Promise<SignInResult | undefined> => {
      setError(undefined);
      setAnnouncement(undefined);
      setSessionExpiredReason(undefined);
      try {
        const result = await authClient.signIn(
          appearance === undefined
            ? {}
            : { stateDecoration: appearanceStateDecoration({ mode: appearance }) },
        );
        setAnnouncement(signInFeedback(result));
        return result;
      } catch (cause) {
        setError(safeAuthErrorMessage(cause));
        return undefined;
      }
    },
    [setAnnouncement, setError, setSessionExpiredReason],
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
  }, [setAnnouncement, setError]);

  const signInSession =
    session === undefined ? undefined : unverifiedSignInSessionFromJwt(session.accessToken);
  return {
    providerLabel: signInAdapter.providerLabel,
    ...(signInSession === undefined ? {} : { session: signInSession }),
    ...(session?.accessToken === undefined ? {} : { accessToken: session.accessToken }),
    ...(session?.subject === undefined ? {} : { subject: session.subject }),
    ready,
    ...(error === undefined ? {} : { error }),
    ...(announcement === undefined ? {} : { announcement }),
    sessionExpired: sessionExpiredReason !== undefined,
    ...(sessionExpiredReason === undefined ? {} : { sessionExpiredReason }),
    signIn,
    signOut,
  };
}
