import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';
{% if context.pwa and not context.web %}import { BrowserSessionClient } from '@baukit/auth-web';
import { Platform } from 'react-native';

import { apiBaseUrl } from '../api-base-url';
{% endif %}
import { authStorage } from '../auth-storage';
import { PRODUCT_NAME } from '../product';
{% if context.pwa and not context.web %}import { browserSignInClient } from './browser-client';
{% endif %}import type { SignInAdapter, SignInClient } from './port';

completeExpoAuthSession();

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['localOidcIssuer'];
const issuer =
  typeof configuredIssuer === 'string' && configuredIssuer !== ''
    ? configuredIssuer
    : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`;

function nativeClient(): SignInClient {
  return createExpoOidcClient(
    {
      issuer,
      clientId: `${PRODUCT_NAME}-mobile`,
      redirectUri: AuthSession.makeRedirectUri({ scheme: PRODUCT_NAME, path: 'oauth' }),
      scopes: [],
      offlineAccess: false,
      storageKeyPrefix: `${PRODUCT_NAME}.local-oidc`,
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
        offlineAccess: false,
        storageKeyPrefix: `${PRODUCT_NAME}:local-oidc`,
      }),
  );
}
{% endif %}
/** Authorization code with PKCE in the system browser against the local Keycloak realm. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'local Keycloak',
  client: {% if context.pwa and not context.web %}Platform.OS === 'web' ? browserClient() : {% endif %}nativeClient(),
};
