import Constants from 'expo-constants';
{% if context.auth_clerk %}import { createClerkExpoClient } from '@baukit/auth-native/clerk-expo';
{% if context.pwa and not context.web %}import { ClerkWebClient } from '@baukit/auth-web/clerk';
import { Platform } from 'react-native';
{% endif %}
import { authStorage } from '../auth-storage';
{% if context.pwa and not context.web %}import { browserSignInClient } from './browser-client';
{% endif %}import type { SignInAdapter } from './port';

const configuredKey: unknown = Constants.expoConfig?.extra?.['clerkPublishableKey'];
const publishableKey = typeof configuredKey === 'string' ? configuredKey : '';

function nativeAdapter(): SignInAdapter {
  const clerk = createClerkExpoClient(publishableKey, authStorage);
  return { providerLabel: 'Clerk', client: clerk.client, Provider: clerk.Provider };
}
{% if context.pwa and not context.web %}
/** The web build uses Clerk's browser SDK, which keeps its own session and no token cache. */
function browserAdapter(): SignInAdapter {
  return {
    providerLabel: 'Clerk',
    client: browserSignInClient(
      () => new ClerkWebClient({ publishableKey, redirectUri: `${window.location.origin}/` }),
    ),
  };
}
{% endif %}
/** Clerk's Expo SDK with hosted sign-in. */
export const signInAdapter: SignInAdapter = {% if context.pwa and not context.web %}Platform.OS === 'web' ? browserAdapter() : {% endif %}nativeAdapter();
{% else %}import * as AuthSession from 'expo-auth-session';
import * as Crypto from 'expo-crypto';
import { completeExpoAuthSession, createExpoOidcEnvironment } from '@baukit/auth-native/expo';
import { createWorkOsNativeClient } from '@baukit/auth-native/workos';
{% if context.pwa and not context.web %}import { WorkOsWebClient } from '@baukit/auth-web/workos';
import { Platform } from 'react-native';
{% endif %}
import { authStorage } from '../auth-storage';
import { PRODUCT_NAME } from '../product';
{% if context.pwa and not context.web %}import { browserSignInClient } from './browser-client';
{% endif %}import type { SignInAdapter, SignInClient } from './port';

completeExpoAuthSession();

function setting(name: string): string {
  const value: unknown = Constants.expoConfig?.extra?.[name];
  return typeof value === 'string' ? value.trim() : '';
}

const apiHostname = setting('workosApiHostname') || 'api.workos.com';

function nativeClient(): SignInClient {
  return createWorkOsNativeClient(
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
  );
}
{% if context.pwa and not context.web %}
/** The web build uses AuthKit's browser SDK, as the web template does (ADR 0006 section 6). */
function browserClient(): SignInClient {
  return browserSignInClient(
    () =>
      new WorkOsWebClient({
        clientId: setting('workosClientId'),
        redirectUri: `${window.location.origin}/`,
        apiHostname,
      }),
  );
}
{% endif %}
/** WorkOS AuthKit as a public PKCE client in the system browser, with rotating refresh tokens in SecureStore. */
export const signInAdapter: SignInAdapter = {
  providerLabel: 'WorkOS',
  client: {% if context.pwa and not context.web %}Platform.OS === 'web' ? browserClient() : {% endif %}nativeClient(),
};
{% endif -%}
