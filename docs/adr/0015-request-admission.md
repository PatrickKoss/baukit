# ADR 0015: Every route sits inside the rate limiter and tenant SQL puts the owner in the WHERE

## Status
Accepted, 2026-11-27.

## Context
A route without a limiter lets one client exhaust token verification, the database or a provider quota. Late routers such as MCP, browser sessions, signed webhooks and `/platform/*` routes are the ones most often merged outside it.

Loading a row by id and comparing the owner in Rust is easy to get wrong in one handler out of many. A query that filters only by id lets any caller who guesses an id read or change another tenant's row.

## Decision
`baukit-ratelimit` supplies the limiter. `layers` wraps a router in the IP and identity scopes, and `authenticated_route_group` adds a per-subject quota for one route group. `RateLimitOptions::from_config` builds the options from `baukit_config::RateLimitConfig`.

The generated backend wires it in `templates/backend/backend/crates/__app__-bin/src/rate_limits.rs`:

- `rate_limited(router, limits)` wraps a router in both scopes.
- `protect_routes(items, late, auth, limits)` runs the IP limiter before `baukit_auth::establish_principal`, so junk bearers are throttled per client before token verification, and the identity limiter after it. Item writes get the `item_writes` group at 30 requests per minute. Every router in `late` is merged behind its own `rate_limited` call.

Tenant SQL names the owner in the predicate, as in `WHERE owner_id = $1 AND id = $2`. The template's `__app__-postgres` adapter does this for every item query.

## Enforced by
- `baukit check`, security rule in `cli/src/check/security.rs`. In a `-bin` crate, a `.merge(x)` whose argument is not wrapped in `rate_limited(` or `protect_routes(` is an error, and so is `axum::serve` without `into_make_service_with_connect_info`. Tests `bin_crate_merges_must_be_rate_limited` and `serve_without_connect_info_is_an_error`. Gate step `baukit-check`.
- `platform_routes_sit_inside_the_rate_limiter` in `templates/platform/backend/tests/platform_integration.rs` and `exhausting_one_client_ip_leaves_another_unthrottled` in `templates/backend/backend/tests/composed/limiter_socket.rs`, run by the fixture gates.
- `item_routes_deny_other_tenants_and_invalid_tokens` in `templates/backend/backend/tests/conformance.rs` calls `baukit_test::assert_cross_tenant_denied` over every OpenAPI operation with a second principal.
- `lint/ast-grep/rules/rust-sql-unscoped-by-id.yml` matches `WHERE id = $1` string literals. Its severity is `hint`, so `baukit check` prints it only with `--verbose` and does not fail. Raising it to `error` would make the owner predicate a hard check.

## Consequences
Products get a limiter on every route by construction and a cross-tenant test that covers new operations without new code. They give up merging routers freely in the bin crate, and global tables need an explicit reason in review because the SQL rule cannot tell them apart.

## History
- 2026-11-07, `213caa32`: the template moved to owner-scoped items and a limiter on every route.
- 2026-11-07, `e5908610`: the ast-grep rules, including `rust-sql-unscoped-by-id`, landed.
- 2026-11-25, `81e2b118`: `compose_api` moved into the bin lib with tests against the composed router.
- The `trusted_proxy_hops` default change is recorded in [ADR 0017](0017-identity-providers-and-bearer-verification.md).
