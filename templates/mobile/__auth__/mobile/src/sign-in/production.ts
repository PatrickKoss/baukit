import Constants from 'expo-constants';
{% if context.auth_clerk %}import { createClerkExpoClient } from '@baukit/auth-native/clerk-expo';

import { authStorage } from '../auth-storage';
import type { SignInAdapter } from './port';

const configuredKey: unknown = Constants.expoConfig?.extra?.['clerkPublishableKey'];
const clerk = createClerkExpoClient(typeof configuredKey === 'string' ? configuredKey : '', authStorage);

/** Clerk's Expo SDK with hosted sign-in. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'Clerk',
  client: clerk.client,
  Provider: clerk.Provider,
};
{% else %}import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
import { completeExpoAuthSession, createExpoOidcEnvironment } from '@baukit/auth-native/expo';
import { createWorkOsNativeClient } from '@baukit/auth-native/workos';

import { authStorage } from '../auth-storage';
import { PRODUCT_NAME } from '../product';
import type { SignInAdapter } from './port';

completeExpoAuthSession();

function setting(name: string): string {
  const value: unknown = Constants.expoConfig?.extra?.[name];
  return typeof value === 'string' ? value.trim() : '';
}

const apiHostname = setting('workosApiHostname') || 'api.workos.com';

/** WorkOS AuthKit as a public PKCE client in the system browser, with rotating refresh tokens in SecureStore. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'WorkOS',
  client: createWorkOsNativeClient(
    {
      issuer: `https://${apiHostname}/`,
      clientId: setting('workosClientId'),
      redirectUri:
        setting('workosRedirectUri') || AuthSession.makeRedirectUri({ scheme: PRODUCT_NAME, path: 'oauth' }),
      scopes: ['openid', 'profile', 'email'],
      offlineAccess: true,
      storageKeyPrefix: `${PRODUCT_NAME}.oidc`,
    },
    createExpoOidcEnvironment({
      randomBytes: (size) => Crypto.getRandomBytesAsync(size),
      storage: authStorage,
    }),
  ),
};
{% endif %}