# Authentication providers

This product selects `{{ context.auth_provider }}`. Backend handlers receive the
same `baukit_auth::Principal` for every provider. `/me` resolves its stable
subject to the product's user UUID. Provider claim checks stay in the verifier.

Configuration loads `config/local.toml`, environment-specific configuration and
`{{ context.app_env }}__AUTH__*` environment variables through `baukit-config`.
Deployment secrets belong in the existing secret store and environment bindings.
Never put an API key in a web or mobile environment file.

## OIDC and external issuers

`--auth oidc` includes a development Keycloak realm. Keycloak runs only on
developer machines and in E2E; Baukit ships no production Keycloak deployment.
A deployed product points the verifier at its external issuer. To use Auth0,
Entra ID, Zitadel, Cognito or Authentik, replace the authentication section:

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

Set `auth.authorized_parties` to the OIDC client IDs allowed to create REST
sessions, for example `["{{ context.app_name }}-web", "{{ context.app_name }}-mobile"]`.
The default is empty for OIDC. A nonempty list calls `OidcConfig::with_allowed_clients`
and requires a matching access-token `azp`. This rejects foreign clients even
when their tokens contain the backend audience. The environment key is
`{{ context.app_env }}__AUTH__AUTHORIZED_PARTIES`, a JSON array. MCP keeps its
separate resource verifier and does not inherit this REST allowlist.

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

{% if context.web or context.pwa %}### Browser sessions

The web client never holds a refresh token (baukit ADR 0006). It redirects to
the IdP with PKCE, then posts the code and verifier to `POST /auth/session/exchange`.
The API redeems them as the `{{ context.app_name }}-web` client, stores the
refresh token encrypted in `baukit_auth_browser_sessions`, and sets an HttpOnly
`SameSite=Strict` cookie. The browser keeps only the access token in memory and
renews it with `POST /auth/session/refresh`.
{% if context.pwa and not context.web %}
The Expo web build of the mobile app is that web client. `Platform.OS === 'web'`
selects `@baukit/auth-web` sign-in (`BrowserSessionClient` for OIDC and the
local realm, `ClerkWebClient` and `WorkOsWebClient` for the hosted providers),
and `mobile/src/auth-storage.ts` refuses every read and write on web. iOS and
Android keep their tokens in SecureStore. `corepack pnpm web` serves the build
on the declared web port, the origin the realm and
`[http] cors_allowed_origins` allow.
{% endif %}
Settings live under `[auth.browser_session]`. Outside local development set
`{{ context.app_env }}__AUTH__BROWSER_SESSION__CREDENTIAL_KEYS` to a
`1:<base64 of 32 random bytes>` keyring, list the web origins in
`{{ context.app_env }}__HTTP__CORS_ALLOWED_ORIGINS`, the callback in
`..__REDIRECT_URIS`, and the browser-facing API origin in `..__PUBLIC_API_ORIGIN`.
Web and API must share a registrable domain, or startup fails. The Helm
`browserSession` values set all of these. Refresh and exchange are rate limited
per IP and per session through the Redis store: 30 requests per minute plus a
burst of 10 per IP, 12 plus 4 per session. `ip_rate_per_minute`, `ip_rate_burst`,
`session_rate_per_minute` and `session_rate_burst` (for example
`{{ context.app_env }}__AUTH__BROWSER_SESSION__IP_RATE_PER_MINUTE`) override
them, and zero per-minute values fail startup. Only `config/local.toml` raises
them, so end-to-end suites that sign in many times from one address pass. Keep
the defaults everywhere else.

{% endif %}## Clerk

Set `auth.issuer` to the Clerk Frontend API URL and `auth.authorized_parties`
to the allowed web origins and mobile authorized-party values. The verifier
checks signature, issuer and expiry, and requires an `azp` from that list.
Clerk omits `azp` when a request has no `Origin`, which is every request from a
native app. `auth.allow_missing_azp`
(`{{ context.app_env }}__AUTH__ALLOW_MISSING_AZP`) accepts such tokens; a
present `azp` must still be in the list. It defaults to
`{{ "true" if context.mobile else "false" }}` for this product. Any Clerk
application on the same instance can mint a token without `azp`, so leave it
off when no native app calls the API.
Clerk session tokens do not require an audience. They authenticate REST,
while MCP uses separate OAuth access tokens. Set `mcp.oauth_client_id` for a
dedicated OAuth client, or `mcp.required_scopes` for clients that register
dynamically; see the remote MCP docs.

Set `VITE_CLERK_PUBLISHABLE_KEY` and `EXPO_PUBLIC_CLERK_PUBLISHABLE_KEY`.
The web adapter uses `@clerk/clerk-js`. Mobile uses `@clerk/expo` hosted sign-in
and SecureStore through `@baukit/auth-native/clerk-expo`. Enable hosted sign-in
in Clerk and register the mobile app's redirect and native identifiers.
See [Clerk Expo](https://clerk.com/docs/reference/expo/overview).

Clerk session tokens carry no `auth_time`, and the erasure fence rejects a
session without a sign-in time after the fence. Add the sign-in time in the
Clerk dashboard under **Sessions**, **Customize session token**, and merge this
claim into any claims already there:

```json
{
  "session_created_at": "{{ '{{' }}session.created_at{{ '}}' }}"
}
```

`session.created_at` is the session's creation time in Unix seconds and stays
the same for the session's life. The verifier reads `session_created_at`
(`baukit_auth::CLERK_SIGN_IN_TIME_CLAIM`) into `Principal::authenticated_at`.
Without the claim, a user who erased their profile stays fenced on every later
sign-in. See [Customize session tokens](https://clerk.com/docs/guides/sessions/customize-session-tokens)
and the [shortcodes](https://clerk.com/docs/guides/sessions/jwt-templates#available-shortcodes).

Set `{{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET` to the Clerk
secret key. The deletion endpoint defaults to `https://api.clerk.com/v1/users`.
A deleted or missing user counts as complete. Provider 5xx and 429 responses
remain pending and retry through the durable erasure worker.

## WorkOS

Set `auth.issuer` to `https://api.workos.com/` (or your configured issuer) and
`auth.client_id` to the AuthKit application client ID. The verifier requires the
signed `client_id` and uses WorkOS's client-specific JWKS endpoint. Session
access tokens need no audience. Set `VITE_WORKOS_CLIENT_ID` and the mobile
`EXPO_PUBLIC_WORKOS_CLIENT_ID`. Enable public-client authentication, configure
redirect URIs and permit the web origin in WorkOS's CORS settings.

The web adapter uses `@workos-inc/authkit-js`. Mobile uses AuthKit PKCE through
the existing native browser and secure-storage ports. It exchanges codes and
rotates refresh tokens through WorkOS's JSON public-client API without an API
key in the app. This follows the protocol in the
[official Expo AuthKit example](https://github.com/workos/expo-authkit-example).

For the native client, register `<app>://oauth` as a redirect URI and a
sign-out redirect in the WorkOS dashboard and keep refresh-token rotation on.
The app reads `EXPO_PUBLIC_WORKOS_CLIENT_ID`, `EXPO_PUBLIC_WORKOS_API_HOSTNAME`
(default `api.workos.com`) and `EXPO_PUBLIC_WORKOS_REDIRECT_URI` (default
`<app>://oauth`). The API key belongs only in the backend's
`{{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET`. The mobile
`check:bundle` fails on an `sk_test_` or `sk_live_` key in the bundle, and
`baukit doctor` fails on one in the mobile env files.

The app keeps the session in SecureStore, refreshes before expiry and on a 401,
and runs one refresh at a time. Each refresh stores the rotated refresh token.
A rejected refresh (`invalid_grant`) ends the session; a network error keeps it.
Logout clears secure storage first, then opens WorkOS's
`/user_management/sessions/logout` with the access token's `sid` and returns to
the app's scheme. A failed logout call still leaves the app signed out.

Set `{{ context.app_env }}__AUTH__IDENTITY_ADMIN_CLIENT_SECRET` to the WorkOS
API key. Deletion uses `https://api.workos.com/user_management/users`.
A 404 counts as complete; provider 5xx and 429 responses use durable retries.
MCP uses WorkOS Connect's issuer and JWKS, distinct from AuthKit session tokens.

{% if context.auth_clerk or context.auth_workos %}## Local development

Production and local development run the same verifier. The bundled Keycloak
realm issues access tokens shaped like {{ "Clerk" if context.auth_clerk else "WorkOS" }} tokens through client
protocol mappers, and `config/local.toml` sets `auth.issuer` and `auth.jwks_uri`
to that realm. Production replaces both through `{{ context.app_env }}__AUTH__ISSUER`
and `{{ context.app_env }}__AUTH__JWKS_URI`; `config/local.toml` loads only in
the local environment.

The realm's public clients, including `{{ context.app_name }}-local`, use only the `basic` client scope with `fullScopeAllowed` off, so their
tokens carry no `aud`. Keycloak sets `sub`, `sid`, `azp`, `iss`, `iat`, `exp`,
`jti`, `typ`, `scope` and `auth_time` itself. Hardcoded-claim mappers add:

{% if context.auth_clerk %}| Claim | Value | Mapper |
| --- | --- | --- |
| `v` | `2` | `clerk session version` |
| `o` | `{"id":"org_local","rol":"admin","slg":"local"}` | `clerk active organization` |

A user-session-note mapper, `clerk sign-in time`, copies Keycloak's `AUTH_TIME`
session note into `session_created_at`, so local tokens reach
`Principal::authenticated_at` through the same claim as production ones.

`azp` is the Keycloak client ID, so `auth.authorized_parties` lists those IDs
locally and the web origins in production. Every local user has the same
active organization. Replace a mapper with a user-attribute mapper when a test
needs per-user organizations.
{% else %}| Claim | Value | Mapper |
| --- | --- | --- |
| `client_id` | `client_{{ context.app_crate }}_local` | `workos client id` |
| `org_id` | `org_local` | `workos organization` |
| `role` | `admin` | `workos role` |
| `permissions` | `["items:read","items:write"]` | `workos permissions` |

`auth.client_id` is `client_{{ context.app_crate }}_local` locally and the
AuthKit client ID in production. Keycloak also adds `azp`, which WorkOS does not
send; the verifier ignores it. Every local user has the same organization.
Replace a mapper with a user-attribute mapper when a test needs per-user
organizations.
{% endif %}
`make keycloak-reconcile` repairs these mappers on a retained volume. The ignored
`local_realm` test signs `test` in through `{{ context.app_name }}-local` in a
disposable Keycloak, checks the claim names, and verifies the token with the
configured verifier and the other provider's.

## Sign-in adapters

The web and mobile apps depend on one sign-in port in `src/sign-in/port.ts`. It
signs in and out, returns the access token, and reads the session: `sub` becomes
`user.id`, `sid` becomes `sessionId`, and the organization comes from
{% if context.auth_clerk %}Clerk's `o` claim (`id`, `rol`, `slg`, and the comma-separated `per`){% else %}WorkOS's `org_id`, `role` and `permissions`{% endif %}.
`unverifiedSignInSessionFromJwt` in `@baukit/api-runtime` builds that shape for
every adapter. It reads claims for display only; the backend verifies the token.

Each app has two adapters. `src/sign-in/local.ts` runs the authorization-code flow
with PKCE S256 against this realm: in the browser through the
`{{ context.app_name }}-web` client, and on mobile through the system browser and the
`{{ context.app_name }}-mobile` client with the `{{ context.app_name }}://oauth` redirect. Sign in as
`test` with `development-password`. `src/sign-in/production.ts` uses the {{ "Clerk" if context.auth_clerk else "WorkOS" }}
SDK described above.

The build picks the adapter. Vite's `--mode production`, the default for
`vite build`, selects the production adapter in `web/src/auth.ts`, and Rollup
drops the local adapter from the bundle. Every other mode, including `vite` and
Vitest, uses the local adapter. On mobile, `metro.config.js` resolves
`src/sign-in/local.ts` to `src/sign-in/production.ts` when `NODE_ENV` is
`production`, which `expo export` and release builds set, unless
`BAUKIT_QA_BUILD=1` marks a QA build that signs in against the local realm.
Jest, TypeScript and `expo start` see the local adapter. No runtime flag can switch
a production bundle back to Keycloak. The QA scripts export `BAUKIT_QA_BUILD` in
their shell. Expo loads every `mobile/.env*` file into each export, so
`metro.config.js` refuses to bundle when one of them sets the flag, and
`baukit doctor` rejects it in `mobile/.env.example` and `mobile/.env`. The app
config passes `EXPO_PUBLIC_LOCAL_OIDC_ISSUER` to the app only in development and
QA builds, and adds `apiBaseUrl` only when `EXPO_PUBLIC_API_URL` is set. The
loopback API fallback lives in the app code under `__DEV__`. The web build picks
its adapter from Vite's `--mode`, which no `web/.env` file can set.

`VITE_LOCAL_OIDC_ISSUER` and `EXPO_PUBLIC_LOCAL_OIDC_ISSUER` override the
realm URL, for example for a physical device on the LAN. Both default to
`http://localhost:{{ context.keycloak_host_port }}/realms/{{ context.app_name }}`.

`pnpm check:bundle` in `web/` and `mobile/` builds production and fails when the
output contains the Keycloak adapter, a `/realms/` URL or a loopback URL. The
mobile check exports Android and iOS{% if context.pwa and not context.web %}, and the Expo web build,{% endif %} with source maps and `--clear`
in a private Metro cache, loads `mobile/.env` as a developer's export does
(the `.env.example` values when no `mobile/.env` exists), scans files copied
from `mobile/public/` as unmapped code, fails if
`src/sign-in/local.ts` is among the bundle's modules, and counts a loopback URL
only when product or `@baukit` code emits it, since React Native and Expo Router
ship their own development-server defaults. The strict gate runs them as the `web-bundle` and `mobile-bundle` steps, and the standard profile's CI runs the mobile check in its `mobile` job.

{% endif %}## Erasure secrets

Set `{{ context.app_env }}__AUTH__ERASURE_HASH_KEY` to at least 32 random bytes.
Keep it stable so erased identities remain fenced. Clerk and WorkOS always
require their deletion API key, including local development. Only the bundled
Keycloak flavor supplies a local admin secret, and only for local development.
Configure `identity_admin_base_url` only for an HTTPS API proxy. The generated
adapter permits loopback HTTP only in local development.
