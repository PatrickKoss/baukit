import Constants from 'expo-constants';
import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
import { completeExpoAuthSession, createExpoOidcClient } from '@baukit/auth-native/expo';

import { authStorage } from '../auth-storage';
import { PRODUCT_NAME } from '../product';
import type { SignInAdapter } from './port';

completeExpoAuthSession();

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['localOidcIssuer'];

/** Authorization code with PKCE in the system browser against the local Keycloak realm. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'local Keycloak',
  client: createExpoOidcClient(
    {
      issuer:
        typeof configuredIssuer === 'string' && configuredIssuer !== ''
          ? configuredIssuer
          : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
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
  ),
};
