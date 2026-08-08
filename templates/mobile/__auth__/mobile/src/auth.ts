import { useCallback, useEffect, useState } from 'react';
import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import * as SecureStore from 'expo-secure-store';
import * as WebBrowser from 'expo-web-browser';

import { logoutUrl, oidcEndpoints } from './auth-urls';

WebBrowser.maybeCompleteAuthSession();

interface StoredTokens {
  readonly accessToken: string;
  readonly refreshToken?: string;
  readonly idToken?: string;
  readonly expiresAt: number;
}

const storageKey = '{{ context.app_name }}:oidc:tokens';
const configuredIssuer: unknown = Constants.expoConfig?.extra?.['oidcIssuer'];
const configuredClientId: unknown = Constants.expoConfig?.extra?.['oidcClientId'];
const issuer =
  typeof configuredIssuer === 'string'
    ? configuredIssuer
    : 'http://localhost:8081/realms/{{ context.app_name }}';
const clientId =
  typeof configuredClientId === 'string' ? configuredClientId : '{{ context.app_name }}-mobile';
const endpoints = oidcEndpoints(issuer);
const discovery: AuthSession.DiscoveryDocument = endpoints;
const redirectUri = AuthSession.makeRedirectUri({ scheme: '{{ context.app_name }}', path: 'oauth' });

export interface OidcAuth {
  readonly accessToken?: string;
  readonly ready: boolean;
  readonly error?: string;
  readonly signIn: () => Promise<void>;
  readonly signOut: () => Promise<void>;
}

export function useOidcAuth(): OidcAuth {
  const [tokens, setTokens] = useState<StoredTokens>();
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [request, response, promptAsync] = AuthSession.useAuthRequest(
    {
      clientId,
      redirectUri,
      responseType: AuthSession.ResponseType.Code,
      scopes: ['openid', 'profile', 'email', 'offline_access'],
      usePKCE: true,
    },
    discovery,
  );

  const storeTokens = useCallback(async (responseTokens: AuthSession.TokenResponse) => {
    const next: StoredTokens = {
      accessToken: responseTokens.accessToken,
      expiresAt: Date.now() + (responseTokens.expiresIn ?? 300) * 1000,
      ...(responseTokens.refreshToken === undefined
        ? {}
        : { refreshToken: responseTokens.refreshToken }),
      ...(responseTokens.idToken === undefined ? {} : { idToken: responseTokens.idToken }),
    };
    await SecureStore.setItemAsync(storageKey, JSON.stringify(next));
    setTokens(next);
  }, []);

  useEffect(() => {
    let active = true;
    void SecureStore.getItemAsync(storageKey).then((raw) => {
      if (!active) {
        return;
      }
      const restored = parseStoredTokens(raw);
      setTokens(restored);
      setReady(true);
    });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (response?.type !== 'success' || request?.codeVerifier === undefined) {
      return undefined;
    }
    const code = response.params['code'];
    if (typeof code !== 'string') {
      const timeout = setTimeout(() => {
        setError('OIDC response did not include an authorization code.');
      }, 0);
      return () => {
        clearTimeout(timeout);
      };
    }
    void AuthSession.exchangeCodeAsync(
      {
        clientId,
        code,
        redirectUri,
        extraParams: { code_verifier: request.codeVerifier },
      },
      discovery,
    )
      .then(storeTokens)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : 'OIDC token exchange failed.');
      });
    return undefined;
  }, [request?.codeVerifier, response, storeTokens]);

  useEffect(() => {
    if (tokens?.refreshToken === undefined) {
      return;
    }
    const delay = Math.max(tokens.expiresAt - Date.now() - 30_000, 0);
    const timeout = setTimeout(() => {
      void AuthSession.refreshAsync(
        { clientId, refreshToken: tokens.refreshToken ?? '' },
        discovery,
      )
        .then(storeTokens)
        .catch((cause: unknown) => {
          setError(cause instanceof Error ? cause.message : 'OIDC token refresh failed.');
        });
    }, delay);
    return () => {
      clearTimeout(timeout);
    };
  }, [storeTokens, tokens]);

  const signIn = useCallback(async () => {
    setError(undefined);
    await promptAsync();
  }, [promptAsync]);

  const signOut = useCallback(async () => {
    const idToken = tokens?.idToken;
    await SecureStore.deleteItemAsync(storageKey);
    setTokens(undefined);
    await WebBrowser.openAuthSessionAsync(
      logoutUrl(endpoints.endSessionEndpoint, clientId, redirectUri, idToken),
      redirectUri,
    );
  }, [tokens?.idToken]);

  return {
    ...(tokens?.accessToken === undefined ? {} : { accessToken: tokens.accessToken }),
    ready: ready && request !== null,
    ...(error === undefined ? {} : { error }),
    signIn,
    signOut,
  };
}

function parseStoredTokens(raw: string | null): StoredTokens | undefined {
  if (raw === null) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(raw);
    if (
      typeof value === 'object' &&
      value !== null &&
      typeof (value as Record<string, unknown>)['accessToken'] === 'string' &&
      typeof (value as Record<string, unknown>)['expiresAt'] === 'number'
    ) {
      return value as StoredTokens;
    }
  } catch {
    // Invalid local state is treated as signed out.
  }
  return undefined;
}
