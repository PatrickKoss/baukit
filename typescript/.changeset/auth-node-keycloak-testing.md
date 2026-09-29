---
'@baukit/auth-node': patch
---

Add the Node-only `@baukit/auth-node/keycloak-testing` subpath for end-to-end tests against a disposable development Keycloak. `keycloakStack(defaults, environment)` reads `E2E_KEYCLOAK_URL`, `E2E_KEYCLOAK_REALM`, `E2E_KEYCLOAK_ADMIN_USERNAME`, `E2E_KEYCLOAK_ADMIN_PASSWORD`, and `E2E_KEYCLOAK_WEB_CLIENT_ID` over the product's defaults. `createKeycloakTestUser(stack, user?, options?)` creates a verified user, random `e2e-<uuid>` by default, and returns its username, password, email, and subject. `revokeKeycloakUserSessions(stack, subject)` ends a user's sessions. `allowKeycloakWebOrigin(stack, origin)` adds an origin to the web client's redirect URIs and web origins when it lacks them. `signInWithKeycloak(page, user, stack, { timeoutMs })` fills the login form by its `#username`, `#password`, and `#kc-login` IDs. Admin calls use `fetch` with a 30-second default timeout and take an injected `fetch`; errors name the step and HTTP status, never a response body. The page parameter is a structural type that a Playwright `Page` satisfies, so the package still has no runtime or peer dependencies.

No breaking changes to existing exports.
