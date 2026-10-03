import { keycloakStack } from '@baukit/auth-node/keycloak-testing';

import { PRODUCT_NAME } from '../../src/product';

/** The composed development Keycloak unless `E2E_KEYCLOAK_*` variables point elsewhere. */
export const stack = keycloakStack({
  url: 'http://localhost:{{ context.keycloak_host_port }}',
  realm: PRODUCT_NAME,
  webClientId: `${PRODUCT_NAME}-web`,
});
