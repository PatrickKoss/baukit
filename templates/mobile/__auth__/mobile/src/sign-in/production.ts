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

const configuredIssuer: unknown = Constants.expoConfig?.extra?.['oidcIssuer'];
const configuredClientId: unknown = Constants.expoConfig?.extra?.['oidcClientId'];

/** WorkOS AuthKit as a public PKCE client in the system browser. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'WorkOS',
  client: createWorkOsNativeClient(
    {
      issuer: typeof configuredIssuer === 'string' ? configuredIssuer : 'https://api.workos.com/',
      clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-mobile`,
      redirectUri: AuthSession.makeRedirectUri({ scheme: PRODUCT_NAME, path: 'oauth' }),
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