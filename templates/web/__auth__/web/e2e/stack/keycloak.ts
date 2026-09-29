import { keycloakStack } from '@baukit/auth-node/keycloak-testing';

/** The composed development Keycloak unless `E2E_KEYCLOAK_*` variables point elsewhere. */
export const stack = keycloakStack({
  url: 'http://localhost:{{ context.keycloak_host_port }}',
  realm: '{{ context.app_name }}',
  webClientId: '{{ context.app_name }}-web',
});
