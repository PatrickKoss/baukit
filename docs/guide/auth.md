# Add sign-in to a product

At the end of this page your product verifies bearer tokens from OIDC, Clerk or WorkOS, keeps web
refresh tokens on the server behind a cookie, signs in native and command-line clients, and runs
against a local Keycloak realm during development. It is for product developers who generated a
backend with `--auth` or who wire `baukit-auth` into an existing service.

## Pick a provider

`baukit new` takes `--auth oidc`, `--auth clerk` or `--auth workos` with any backend, web or mobile
flavor. Every choice returns the same `baukit_auth::Principal` to handlers, so product code does not
branch on the provider.

```sh
baukit new my-product --backend --web --mobile --auth oidc
```

- `oidc` works with any standards-compliant issuer: Keycloak, Auth0, Entra ID, Zitadel, Cognito or
  Authentik. The verifier is `OidcVerifier`, configured from `OidcConfig`.
- `clerk` verifies Clerk session tokens with `ClerkVerifier`. Clerk tokens may omit `aud`, so the
  verifier binds them to an allowlist of authorized parties.
- `workos` verifies AuthKit session tokens with `WorkOsVerifier`, bound to one Application through the
  signed `client_id` claim.

`MultiIssuerVerifier::from_verifiers` accepts tokens from several issuers in one service and routes
each token to exactly one verifier by its `iss`. The constructors and their options are in the
[baukit-auth README](../../rust/crates/baukit-auth/README.md#clerk-and-workos), which compiles every
snippet as a doctest. The generated product's `docs/auth-providers.md` has the per-provider setup:
issuer strings, audiences, client registration and the web and mobile environment variables.

## Verify tokens in the backend

The minimal wiring, from the `baukit-auth` README doctest:

```rust,no_run
use axum::{Router, routing::get};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = OidcConfig::keycloak("https://identity.example.com", "products", "orders-api")?;
let auth = AuthState::new(OidcVerifier::discover(config).await?);

async fn me(principal: Principal) -> String {
    principal.subject().to_owned()
}

let _app: Router = Router::new().route("/me", get(me)).with_state(auth);
# Ok(())
# }
```

A handler that takes `Principal` never runs for an unauthenticated request. A generated backend
loads its settings from `[auth]` in `config/local.toml` and from `<PREFIX>__AUTH__*` environment
variables, where `<PREFIX>` is the product's upper-case environment prefix.

### Restrict the clients with `authorized_parties`

An audience check alone lets any client whose token carries the backend audience call the API. Set
`auth.authorized_parties` to the client IDs allowed to create REST sessions:

```toml
[auth]
authorized_parties = ["my-product-web", "my-product-mobile"]
```

For OIDC a nonempty list calls `OidcConfig::with_allowed_clients`, and a token whose `azp` is
missing or not listed fails. The list is empty by default. The environment key is
`<PREFIX>__AUTH__AUTHORIZED_PARTIES`, a JSON array. Use client IDs, not web origins.

Clerk uses the same key differently. `ClerkVerifier` compares `azp` against the list, which holds
web origins in production and the Keycloak client IDs locally. `auth.allow_missing_azp` accepts Clerk
tokens without `azp`, which native Clerk sessions need. The generated `config/local.toml` sets it to
`true` only when the product has a mobile app.

The MCP endpoint has its own verifier and does not read this list. See [the MCP guide](mcp.md).

### Outages are not sign-outs

When the issuer, its JWKS or the token store is unreachable, verification fails as unavailable
rather than as an invalid token. The Axum extractor then answers 503 with the code `unavailable`
and `Retry-After: 5`, without `WWW-Authenticate`, so clients retry instead of signing in again. The [verification outages](../../rust/crates/baukit-auth/README.md#verification-outages)
section of the README lists which failures count.

## Keep web refresh tokens on the server

A browser that stores a refresh token gives it to any script that runs on the page. baukit avoids
that with a backend-for-frontend, decided in
[ADR 0006](../adr/0006-browser-token-handling.md). The browser runs the authorization code flow with
PKCE against the IdP and posts the code and verifier to the API. The API redeems them as the same
OIDC client, stores the refresh and ID tokens encrypted in PostgreSQL, and sets an `HttpOnly`,
`SameSite=Strict` cookie holding `<session id>.<secret>`. The browser keeps only the short-lived
access token in memory.

The routes live under `/auth/session`:

- `POST /auth/session/exchange` redeems the code.
- `POST /auth/session/refresh` returns a new access token and rotates the cookie. A rotated cookie
  presented after its 10 second grace window revokes the whole session family.
- `POST /auth/session/logout` revokes the refresh token at the IdP and returns its end-session URL
  when it has one.

Every route requires the `X-Baukit-Session: 1` header and an `Origin` from the CORS allowlist.

### What the template renders

A product generated with `--auth oidc` and `--web` or `--pwa` gets the whole setup with the
browser session turned on. The other flavors get the same code with it turned off by default.

| Piece | Generated file |
| --- | --- |
| `baukit-auth` with the `browser-session` feature, and `baukit-ratelimit` with its `browser-session` feature | `backend/crates/<app>-bin/Cargo.toml` |
| `BrowserSessionSettings` and `browser_session_router` | `backend/crates/<app>-bin/src/browser_session.rs` |
| The session table migrations, copied from the crate | `backend/migrations/0008_baukit_auth_browser_sessions.sql`, `backend/migrations/0010_baukit_auth_browser_session_record_ids.sql` |
| Local settings with raised rate limits | `config/local.toml`, section `[auth.browser_session]` |
| Chart values | `deploy/values.yaml`, section `browserSession` |
| The web client | `web/src/auth.ts` |

The template sources are under `templates/backend/__auth__/` and `templates/web/__auth__/`.

### The backend feature and its migrations

Enable the feature in the crate that builds the router:

```toml
baukit-auth = { workspace = true, features = ["browser-session"] }
```

The feature pulls in `sqlx-postgres` and `baukit-credential-vault`. Two migrations create and extend
the session table. The crate ships them as
`rust/crates/baukit-auth/migrations/0002_baukit_auth_browser_sessions.sql` and
`0003_baukit_auth_browser_session_record_ids.sql`, and exports them as
`POSTGRES_BROWSER_SESSIONS_MIGRATION_SQL` and `POSTGRES_BROWSER_SESSION_RECORD_IDS_MIGRATION_SQL`.
A generated product copies them into its own numbered sequence. When you add them by hand, follow
the order in `rust/migrations.toml` and the rules in
[authoring SQL migrations](../migrations/authoring-sql-migrations.md).

The router comes from `baukit_auth::browser_session`. This is the core of
`templates/backend/__auth__/backend/crates/__app__-bin/src/browser_session.rs`:

```rust
let mut config = BrowserSessionConfig::new(
    dependencies.issuer,
    settings.client_id.clone(),
    settings.cookie_name.clone(),
    &settings.public_api_origin,
    CredentialCipher::parse(&keys)?,
)?
.with_redirect_uris(settings.redirect_uris.iter().cloned())
.with_idle_timeout(Duration::from_secs(settings.idle_timeout_seconds))?
.with_absolute_timeout(Duration::from_secs(settings.absolute_timeout_seconds))?
.with_insecure_local_cookie(settings.insecure_local_cookie)
.with_rate_limits(settings.rate_limits())?;
```

`BrowserSessionState::discover(config, http, pool, limiter)` reads the IdP discovery document, and
`browser_session::router(state)` returns the routes. `with_internal_issuer` points the backend at a
realm URL that only the backend can reach, which Compose uses for `http://keycloak:8080`.
`CredentialCipher::parse` takes the keyring that encrypts the stored tokens, written as
`version:base64key,...`.

The generated settings default to a 30 day idle timeout and a 90 day absolute timeout. Outside local
development the product refuses to start without `credential_keys` or without the shared Redis
rate-limit store. The rate limits from ADR 0006 are 30 requests per minute plus a burst of 10 per
client IP, and 12 plus 4 per session. Only `config/local.toml` raises them, so end-to-end suites that
sign in many times from one address pass.

Erasure and cleanup are wired too. `PostgresProfileErasure` deletes the subject's session rows, and
with `--worker` the durable worker calls `purge_browser_sessions` daily. Without the worker, call
`baukit_auth::browser_session::purge_browser_sessions` from a daily scheduled command.

### Chart values

The `baukit-app` chart turns the settings into environment variables. Its defaults, from
`deploy/chart/baukit-app/values.yaml`:

```yaml
browserSession:
  enabled: false
  allowedOrigins: []
  redirectUris: []
  postLogoutRedirectUri: ""
  publicApiOrigin: ""
  credentialKeysSecretRef:
    name: ""
    key: keys
```

With `enabled: true` the chart sets `AUTH__BROWSER_SESSION__ENABLED`,
`AUTH__BROWSER_SESSION__REDIRECT_URIS`, `AUTH__BROWSER_SESSION__PUBLIC_API_ORIGIN` and, when set,
`AUTH__BROWSER_SESSION__POST_LOGOUT_REDIRECT_URI`. It reads the keyring from the named Secret into
`<PREFIX>__AUTH__BROWSER_SESSION__CREDENTIAL_KEYS`. `allowedOrigins` becomes
`HTTP__CORS_ALLOWED_ORIGINS`, so the runtime Secret must not set CORS origins as well. Rendering
fails when `allowedOrigins`, `redirectUris`, `publicApiOrigin` or the Secret name is empty. The web
and API hosts must share a registrable domain, or the backend fails at startup.

Create the keyring Secret before the release. A keyring with one key looks like
`1:<base64 of 32 random bytes>`. Add a new version in front to rotate keys.

### The web client

`@baukit/auth-web` holds the browser half. `BrowserSessionClient` runs the redirect, calls the
three routes, and keeps the access token in memory. The generated `web/src/auth.ts`
(`templates/web/__auth__/web/src/auth.ts`) builds it from Vite variables:

```ts
new BrowserSessionClient({
  apiBaseUrl,
  issuer:
    typeof configuredIssuer === 'string'
      ? configuredIssuer
      : `http://localhost:8081/realms/${PRODUCT_NAME}`,
  clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-web`,
  redirectUri: `${window.location.origin}/`,
  // scopes, audience and resource follow
});
```

Call `handleCallback()` after the IdP redirect, `restore()` on load, and `getAccessToken()` before
each API call. Only a 401 from refresh signs the browser out. Network errors, 429 and 5xx reject
with a retryable error and keep the account hint, so an offline reload still opens the right local
data. Session calls are the only requests sent with `credentials: 'include'`. The
[auth-web README](../../typescript/packages/auth-web/README.md) documents tabs, storage keys and
failure handling.

Clerk and WorkOS web apps use the provider SDK in production builds. Their development builds sign
in against the local Keycloak realm through the same `/auth/session` routes, which is why
`config/local.toml` enables the browser session for every provider. Hosted-session providers are
out of scope for the backend-for-frontend in production, as ADR 0006 section 6 explains.

## Sign in native apps and command-line tools

Mobile apps use `@baukit/auth-native`. `createExpoOidcClient` runs authorization code with PKCE
through `expo-auth-session`, keeps the session in `expo-secure-store`, rotates refresh tokens and
takes the subject from UserInfo, never from an unverified ID token. Clerk and WorkOS have their own
adapters in the same package. See the
[auth-native README](../../typescript/packages/auth-native/README.md).

Command-line tools and local agents use the device flow in `@baukit/auth-node`.
`DeviceFlowClient` from `@baukit/auth-node/device-flow` shows a verification URL and user code,
caches tokens per namespace and can take a token from the environment instead. See the
[auth-node README](../../typescript/packages/auth-node/README.md#device-flow-client).

## Accept personal access tokens

A cron job or a script cannot run an interactive login. `ApiTokenService` issues personal access
tokens, stores only their SHA-256 digest and compares in constant time. `ApiTokenVerifier` wraps
the OIDC verifier, so one `Authorization: Bearer` header accepts both kinds and handlers read
`Principal::api_token` and `Principal::grants` when they need to tell them apart.

From the README doctest:

```rust
let tokens = ApiTokenService::with_format(store, ApiTokenFormat::new("acme_")?);

// Return the secret in the creation response; it cannot be recovered later.
let issued = tokens.issue(owner_id, NewApiToken::new("CI deploy")).await?;
assert!(issued.secret.starts_with("acme_"));

let _auth = AuthState::new(ApiTokenVerifier::new(tokens, oidc));
```

No template generates token routes. Add the table from
`rust/crates/baukit-auth/migrations/0001_baukit_auth_api_tokens.sql`, use `PostgresApiTokenStore`
behind the `sqlx-postgres` feature, and write the issue and revoke routes in the product. Every such
route needs a test that a second principal cannot list or revoke the first one's tokens. For tokens
at `/mcp`, see [personal access tokens in the MCP guide](mcp.md#personal-access-tokens).

## Run a local Keycloak

Every `--auth` flavor renders a development Keycloak realm, not only OIDC. The realm lives in
`keycloak/realm.json`, `keycloak/realm-policy.json` and `keycloak/reconcile.json`, and Compose starts
Keycloak on host port 8081 unless the product sets a port offset.

- For `oidc`, the realm is the issuer the product uses locally.
- For `clerk` and `workos`, client protocol mappers make the realm issue tokens shaped like the
  provider's. `config/local.toml` points `auth.issuer` and `auth.jwks_uri` at the realm, so local
  development runs the production verifier. Production replaces both through environment variables.

The commands, from the generated `Makefile`:

```sh
make dev                # start Keycloak and Redis, check the realm policy, reconcile the realm
make db-up              # start PostgreSQL
make keycloak-policy    # check the realm against keycloak/realm-policy.json
make keycloak-reconcile # repair a retained Keycloak volume
```

Sign in as `test` with the password `development-password`. These credentials exist only in the
development realm. Keycloak runs on developer machines and in end-to-end tests. baukit ships no
production Keycloak deployment, so a deployed product points at an external issuer. The generated
`docs/auth-providers.md` explains how to remove the local realm when you use one.

## Reference

- [baukit-auth README](../../rust/crates/baukit-auth/README.md)
- [`@baukit/auth-web` README](../../typescript/packages/auth-web/README.md)
- [`@baukit/auth-native` README](../../typescript/packages/auth-native/README.md)
- [`@baukit/auth-node` README](../../typescript/packages/auth-node/README.md)
- [ADR 0006: browser token handling](../adr/0006-browser-token-handling.md)
- [ADR 0017: identity providers and bearer verification](../adr/0017-identity-providers-and-bearer-verification.md)
- [Template provider setup](../../templates/backend/__auth__/docs/auth-providers.md)
- [Template browser session module](../../templates/backend/__auth__/backend/crates/__app__-bin/src/browser_session.rs)
- [Template local configuration](../../templates/backend/__auth__/config/local.toml)
- [Chart values](../../deploy/chart/baukit-app/values.yaml)
- [Authoring SQL migrations](../migrations/authoring-sql-migrations.md)
- [Auth verifier hardening migration](../migrations/auth-verifier-hardening.md)
- [The MCP guide](mcp.md)
- [The privacy guide](privacy.md)
