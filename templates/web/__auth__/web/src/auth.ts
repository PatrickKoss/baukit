{% if context.auth_oidc %}import { BrowserSessionClient } from '@baukit/auth-web';

import { apiBaseUrl } from './api-base-url';
import { PRODUCT_NAME } from './product';
import type { SignInPort } from './sign-in/port';
import { createSignInPort } from './sign-in/provider-client';

const configuredIssuer: unknown = import.meta.env.VITE_OIDC_ISSUER;
const configuredClientId: unknown = import.meta.env.VITE_OIDC_CLIENT_ID;
const configuredAudience: unknown = import.meta.env.VITE_OIDC_AUDIENCE;
const configuredResource: unknown = import.meta.env.VITE_OIDC_RESOURCE;
const configuredScopes: unknown = import.meta.env.VITE_OIDC_SCOPES;
const offlineAccess = import.meta.env.VITE_OIDC_OFFLINE_ACCESS !== 'false';

export const authClient: SignInPort = createSignInPort(
  'local Keycloak',
  () =>
    new BrowserSessionClient({
      apiBaseUrl,
      issuer:
        typeof configuredIssuer === 'string'
          ? configuredIssuer
          : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
      clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-web`,
      redirectUri: `${window.location.origin}/`,
      scopes:
        typeof configuredScopes === 'string' && configuredScopes.trim()
          ? configuredScopes.trim().split(/\s+/u)
          : ['openid', 'profile', 'email'],
      ...(typeof configuredAudience === 'string' && configuredAudience.trim()
        ? { audience: configuredAudience }
        : {}),
      ...(typeof configuredResource === 'string' && configuredResource.trim()
        ? { resource: configuredResource }
        : {}),
      offlineAccess,
      storageKeyPrefix: `${PRODUCT_NAME}:oidc`,
    }),
);
{% else %}import type { SignInPort } from './sign-in/port';
import { createLocalSignIn } from './sign-in/local';
import { createProductionSignIn } from './sign-in/production';

// Vite replaces MODE at build time, so a production bundle drops the local adapter.
export const authClient: SignInPort =
  import.meta.env.MODE === 'production' ? createProductionSignIn() : createLocalSignIn();
{% endif %}