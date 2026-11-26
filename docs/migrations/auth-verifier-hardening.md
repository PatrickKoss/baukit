# Auth verifier hardening

`baukit-auth` rejects access tokens that carry `nonce`, and the JWKS cache
has two new settings.

## Restricting clients stays opt-in

Access tokens are checked against the audience, as RFC 9068 describes. `azp`
names the requesting client, so the verifier matches it against a list only
when you call `with_allowed_clients`. Without one, a multi-audience token such
as Keycloak's default `aud: ["app-backend", "account"]` needs a string `azp`
of any value, as before.

Generated products set `auth.authorized_parties` in `config/local.toml` to
their web, mobile and local client IDs. Set the same list in every other
environment:

```rust
OidcConfig::new(issuer, "api")?.with_allowed_clients(["app-web", "app-mobile"])?
```

## `nonce` on the access path

`verify` now rejects a token with a `nonce` claim with
`VerificationError::WrongTokenType`. That claim marks an ID token. Send the
access token to APIs, and verify ID tokens with `verify_id_token`. Code that
matches `VerificationError` exhaustively needs the new variant.

To also require `typ: at+jwt` (RFC 9068), call
`OidcConfig::requiring_access_token_type()`. It is off by default because
several IdPs send `typ: JWT`.

## JWKS cooldown and stale grace

All refetches share one cooldown, set with
`with_jwks_refresh_cooldown(Duration)`. The default is 30 s and zero is
rejected. A key rotated at the IdP becomes usable once the cooldown since the
last fetch ends. Tests that rotate keys right after the first fetch should set
a short cooldown.

When a refetch fails, the verifier serves the last good set for
`with_jwks_stale_grace(Duration)`, default one hour, past its TTL.

0.31.0 shipped a 10 minute default. The release after it raises the default
to one hour, so a longer IdP outage does not reject every token whose key the
verifier already knows. A key the IdP revoked stays usable for up to that
hour while refetches fail. Products that need the shorter window set
`with_jwks_stale_grace(Duration::from_secs(600))`.

## BFF session response

`POST` responses from the session router carry `expiresIn` in seconds next to
`expiresAt`. `@baukit/auth-web` uses it so a skewed browser clock no longer
breaks refresh timing. No action is needed unless you parse the response
yourself.
