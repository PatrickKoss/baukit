import { OidcClient, type OidcClientEnvironment } from '@baukit/auth-web';

import { PRODUCT_NAME } from '../product';
import type { SignInPort } from './port';
import { createSignInPort } from './provider-client';

const configuredIssuer: unknown = import.meta.env['VITE_LOCAL_OIDC_ISSUER'];

/** Authorization code with PKCE against the product's local Keycloak realm. */
export function createLocalSignIn(environment?: OidcClientEnvironment): SignInPort {
  return createSignInPort('local Keycloak', () => {
    const config = {
      issuer:
        typeof configuredIssuer === 'string' && configuredIssuer !== ''
          ? configuredIssuer
          : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
      clientId: `${PRODUCT_NAME}-web`,
      redirectUri: `${window.location.origin}/`,
      scopes: [],
      offlineAccess: false,
      storageKeyPrefix: `${PRODUCT_NAME}:local-oidc`,
    };
    return environment === undefined ? new OidcClient(config) : new OidcClient(config, environment);
  });
}
