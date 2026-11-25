# Stricter security suites and a shared `compose_api`

`baukit-test` now probes more of the API and fails on gaps it used to skip. The
backend template moved API composition out of `bin/api.rs` into the bin crate's
library, so tests run against the router the binary serves.

## `baukit-test` changes

`check_auth_negative_suite` fails on every operation the OpenAPI document lists
without security unless the case declares it public:

```rust
AuthNegativeCase::new(&issuer, AUDIENCE)
    .public("get", "/health", "liveness probe for the load balancer")
```

A `.public(...)` entry that names a secured operation also fails, so stale
entries surface.

`InvalidCredential::ALL` has 11 cases instead of 5. The new ones are a wrong
issuer, a token that is not valid yet, an HS256 token keyed with the issuer's
public JWK, a random `kid`, two `Authorization` headers, and a `Basic` header.
Code that matched on `InvalidCredential` exhaustively needs the new arms.

The suite also sends one valid token per operation. A route that answers 401 to
it fails, because a route that rejects everyone proves nothing about auth. A 401
for a missing token must carry `WWW-Authenticate`.

`check_cross_tenant_denied` now probes every operation, including the ones
without path parameters. Those only fail when a leak marker shows up in the
body. Add `.owner(token)` to the case to make the owner read each resource
before and after the other tenant's request. The owner must get a 2xx, and a
changed body after a denied `DELETE` or `PATCH` fails the suite. Exclude
operations that act on the caller's own account, such as `DELETE /me`.

`OperationFixtures::request` takes the raw `Authorization` header values
instead of a token.

## Generated backends

`compose_api(&config, &ApiDependencies)` lives in `<app>-bin/src/compose.rs`
and returns `ComposedApi { router, late_routes }`. `late_routes` lists the
routes merged outside the OpenAPI document (browser session, MCP, platform
inbound). Products that edited `compose_api` in `bin/api.rs` should move the
edit into `compose.rs` and build an `ApiDependencies` in `api.rs`.

`protect_routes` now runs an IP-only rate limiter before token verification
and the identity limiter after it, so invalid bearers cannot spend verifier
work beyond the IP allowance.

New tests live in `backend/tests/composed/`. They need `proptest` as a
dev-dependency of the bin crate and a `[[test]] name = "composed"` entry; copy
both from the template.

## Chart

`opsService.type` accepts only `ClusterIP`. `config.overrides` sets
`RATE_LIMIT__TRUSTED_PROXY_HOPS: "1"` for one Traefik hop; change it if your
ingress path has a different number of proxies.
