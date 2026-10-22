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

The web `VITE_OIDC_AUDIENCE`, `VITE_OIDC_RESOURCE` and space-separated
`VITE_OIDC_SCOPES` configure authorization requests. Mobile uses the matching
`EXPO_PUBLIC_OIDC_*` variables. Set `OIDC_OFFLINE_ACCESS=false` under each prefix
if the provider issues refresh tokens without the `offline_access` scope.
Restart Expo after changing its configuration.

For Auth0, register an API identifier, set backend `auth.audience` to it, and
request that identifier through `OIDC_AUDIENCE`. Without a custom API audience,
Auth0 can return an opaque access token. See [Auth0 access tokens](https://auth0.com/docs/secure/tokens/access-tokens/get-access-tokens).
For Cognito, use the user-pool issuer for discovery and a public app client with
managed login. Set `OIDC_RESOURCE` and backend `auth.audience` to the same API
URL. Disable the `offline_access` scope. Cognito adds `aud` only when the client
requests resource binding; refreshed tokens retain it.
See [Cognito resource binding](https://docs.aws.amazon.com/cognito/latest/developerguide/authorization-endpoint.html).
For Entra ID, request the registered API's delegated scope in `OIDC_SCOPES` and
use its access-token audience and exact tenant issuer in backend configuration.
Use the API application's client ID as the audience for v2 access tokens, and
use discovery metadata for that token version. See
[Entra access tokens](https://learn.microsoft.com/en-us/entra/identity-platform/access-tokens).
For Zitadel, select JWT access tokens and add
`urn:zitadel:iam:org:project:id:YOUR_PROJECT_ID:aud` to `OIDC_SCOPES`.
Set backend `auth.audience` to that project ID. See
[Zitadel scopes](https://zitadel.com/docs/apis/openidoauth/scopes).
For Authentik, keep its default per-application issuer, select an asymmetric
signing key and include the `offline_access` scope mapping for refresh tokens.
Set backend `auth.audience` to the audience issued for the application.
See [Authentik OAuth configuration](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/).

Remove the `keycloak` service, its volume and `keycloak/` tree when using an
external issuer. If you use the optional Compose backend profile, also remove
its Keycloak dependency and set `AUTH__JWKS_URI` and `MCP__JWKS_URI` under the
product prefix to that provider's key endpoints. Keep both issuers public.
Start Redis with `docker compose up -d redis` and PostgreSQL
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
