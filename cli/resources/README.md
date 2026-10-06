# Keycloak client scopes

`remote-mcp-client-scopes.json` contains the standard OIDC client scopes from
Keycloak 26.8.0 and the example `items:read` scope. Realm-specific IDs are omitted.

An explicit `clientScopes` list in a realm import replaces the built-in scopes.
The remote template keeps `basic`, which supplies the access token's `sub`
claim, and the standard scopes used by the backend, web, and mobile clients.

When changing the Keycloak image version, export OIDC client scopes from a
fresh realm and compare their mapper settings. Keep `items:read`, run the
generator tests, and run `make mcp-fixture-gate` to verify real tokens.
