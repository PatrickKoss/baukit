import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';
{% if context.pwa and not context.web %}import { BrowserSessionClient } from '@baukit/auth-web';
import { Platform } from 'react-native';

import { apiBaseUrl } from '../api-base-url';
{% endif %}
import { appScheme } from '../app-scheme';
import { authStorage } from '../auth-storage';
import { PRODUCT_NAME } from '../product';
{% if context.pwa and not context.web %}import { browserSignInClient } from './browser-client';
{% endif %}import type { SignInAdapter, SignInClient } from './port';

completeExpoAuthSession();

function setting(name: string): string {
  const value: unknown = Constants.expoConfig?.extra?.[name];
  if (typeof value !== 'string' || value === '') throw new Error(`APP_VARIANT builds need extra.${name} in the Expo config`);
  return value;
}

const issuer = setting('labOidcIssuer');
const clientId = setting('labOidcClientId');

function nativeClient(): SignInClient {
  return createExpoOidcClient(
    {
      issuer,
      clientId,
      redirectUri: AuthSession.makeRedirectUri({ scheme: appScheme(), path: 'oauth' }),
      scopes: [],
      offlineAccess: true,
      storageKeyPrefix: `${PRODUCT_NAME}.lab-oidc`,
    },
    {
      randomBytes: (size) => Crypto.getRandomBytesAsync(size),
      storage: authStorage,
    },
  );
}
{% if context.pwa and not context.web %}
/** The web build redeems the code through the backend's `/auth/session`, which keeps the refresh token. */
function browserClient(): SignInClient {
  return browserSignInClient(
    () =>
      new BrowserSessionClient({
        apiBaseUrl: apiBaseUrl || window.location.origin,
        issuer,
        clientId: `${PRODUCT_NAME}-web`,
        redirectUri: `${window.location.origin}/`,
        scopes: [],
        offlineAccess: true,
        storageKeyPrefix: `${PRODUCT_NAME}:lab-oidc`,
      }),
  );
}
{% endif %}
/** Authorization code with PKCE in the system browser against the lab realm the build names. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'the lab realm',
  client: {% if context.pwa and not context.web %}Platform.OS === 'web' ? browserClient() : {% endif %}nativeClient(),
};
