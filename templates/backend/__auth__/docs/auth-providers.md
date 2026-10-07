# Authentication providers

This product selects `{{ context.auth_provider }}`. Backend handlers receive the
same `baukit_auth::Principal` for every provider. `/me` resolves its stable
subject to the product's user UUID. Provider claim checks stay in the verifier.

Configuration loads `config/local.toml`, environment-specific configuration and
`{{ context.app_env }}__AUTH__*` environment variables through `baukit-config`.
Deployment secrets belong in the existing secret store and environment bindings.
Never put an API key in a web or mobile environment file.

## OIDC and external issuers

`--auth oidc` includes a development Keycloak realm. To use Auth0, Entra ID,
Zitadel, Cognito or Authentik, replace the authentication section:

```toml
[auth]
provider = "oidc"
issuer = "https://identity.example.com/tenant"
audience = "{{ context.app_name }}-backend"
```

Copy the exact issuer from the provider's discovery document, including its
path and trailing slash. Create public authorization-code clients with PKCE S256.
Grant the backend audience in access tokens. ID tokens do not authenticate the
API. Configure the web `VITE_OIDC_ISSUER` and `VITE_OIDC_CLIENT_ID`, and mobile
`EXPO_PUBLIC_OIDC_ISSUER` and `EXPO_PUBLIC_OIDC_CLIENT_ID` with those clients.
Register the generated web callback and mobile `{{ context.app_name }}://oauth`
redirect. The native adapter needs UserInfo and supports refresh-token rotation.
The backend discovers JWKS at startup. `auth.jwks_uri` explicitly overrides
that endpoint for a proxy or test; issuer and audience checks still apply.

Remove the `keycloak` service, its volume and `keycloak/` tree when using an
external issuer. Start Redis with `docker compose up -d redis` and PostgreSQL
with `make db-up`. Do not run the Keycloak reconciliation targets. The generated
`provider_config` test loads an external issuer from TOML and calls the verifier
and authenticated product route with its signed token.

OIDC defines no account-deletion API. The OIDC flavor's deletion adapter is
Keycloak-specific. For another issuer, supply its `IdentityAccountDeleter` in
`identity_erasure`; do not point the Keycloak Admin API adapter at its OIDC URL.
The product erasure transaction, fences and durable worker stay the same.

## Clerk

Set `auth.issuer` to the Clerk Frontend API URL and `auth.authorized_parties`
to the allowed web origins and mobile authorized-party values. The verifier
checks signature, issuer and expiry; any present `azp` must match that list.
Clerk session tokens do not require an audience. They authenticate REST,
while MCP uses separate OAuth access tokens and a dedicated OAuth client.

Set `VITE_CLERK_PUBLISHABLE_KEY` and `EXPO_PUBLIC_CLERK_PUBLISHABLE_KEY`.
The web adapter uses `@clerk/clerk-js`. Mobile uses `@clerk/expo` hosted sign-in
and SecureStore through `@baukit/auth-native/clerk-expo`. Enable hosted sign-in
in Clerk and register the mobile app's redirect and native identifiers.
See [Clerk Expo](https://clerk.com/docs/reference/expo/overview).

Set `{{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET` to the Clerk
secret key. The deletion endpoint defaults to `https://api.clerk.com/v1/users`.
A deleted or missing user counts as complete. Provider 5xx and 429 responses
remain pending and retry through the durable erasure worker.

## WorkOS

Set `auth.issuer` to `https://api.workos.com/` (or your configured issuer) and
`auth.client_id` to the AuthKit application client ID. The verifier requires the
signed `client_id` and uses WorkOS's client-specific JWKS endpoint. Session
access tokens need no audience. Set `VITE_WORKOS_CLIENT_ID` and the mobile
`EXPO_PUBLIC_OIDC_CLIENT_ID`. Enable public-client authentication, configure
redirect URIs and permit the web origin in WorkOS's CORS settings.

The web adapter uses `@workos-inc/authkit-js`. Mobile uses AuthKit PKCE through
the existing native browser and secure-storage ports. It exchanges codes and
rotates refresh tokens through WorkOS's JSON public-client API without an API
key in the app. This follows the protocol in the
[official Expo AuthKit example](https://github.com/workos/expo-authkit-example).
Logout clears secure storage before ending the provider session.

Set `{{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET` to the WorkOS
API key. Deletion uses `https://api.workos.com/user_management/users`.
A 404 counts as complete; provider 5xx and 429 responses use durable retries.
MCP uses WorkOS Connect's issuer and JWKS, distinct from AuthKit session tokens.

## Erasure secrets

Set `{{ context.app_env }}__AUTH__ERASURE_HASH_KEY` to at least 32 random bytes.
Keep it stable so erased identities remain fenced. Clerk and WorkOS always
require their deletion API key, including local development. Only the bundled
Keycloak flavor supplies a local admin secret. Production always requires an
explicit secret. Configure `identity_admin_base_url` only for an HTTPS API
proxy. The generated adapter permits loopback HTTP only in local development.
