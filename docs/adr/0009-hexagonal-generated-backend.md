# ADR 0009: Generated backends are hexagonal crates, wired in one compose_api function

## Status
Accepted, 2026-11-27.

## Context

Every generated product starts from the backend template. Whatever shape the template has, each
product keeps, because nobody restructures a backend once routes and tables exist. A single crate
lets handlers call SQLx directly, and then domain tests need a database.

Products also need one place where configuration turns into a running router. Before it existed,
each `main` built its own stack, and security layers went missing in some of them.

## Decision

The template backend under `templates/backend/backend/crates/` is a Cargo workspace of six crates:

- `<app>-domain` holds types and rules. It depends on `baukit-core`, serde, serde_json, thiserror and
  uuid only.
- `<app>-ports` holds the traits the services call. It depends on the domain.
- `<app>-services` orchestrates use cases against the ports.
- `<app>-postgres` holds the SQLx adapters.
- `<app>-api` holds axum routes and OpenAPI, with `baukit-http` and `baukit-openapi`.
- `<app>-bin` composes it all. `src/compose.rs` has `compose_api(config, deps)`, which returns a
  `ComposedApi` with the router and its layers, and is async when auth is on. `rate_limits.rs`
  holds the route groups, and `bin/api.rs` serves the result.

The template also ships its own ADR, `templates/backend/docs/adr/0001-backend-boundaries.md`.
Products that predate `compose_api` move with
[docs/migrations/security-suites-and-composed-api.md](../migrations/security-suites-and-composed-api.md).

## Enforced by

- The template Cargo manifests. The domain crate cannot import SQLx or axum because it does not
  depend on them, and the generated fixture gate builds it.
- `cli/tests/generator.rs`: `backend_generation_matches_golden_tree_and_is_deterministic` pins every
  generated file.
- The `security` rule of `baukit check`. In bin crates, a router `.merge(` must sit inside
  `rate_limited(` or `protect_routes(`.
- The template tests `backend/tests/composed` and `conformance.rs` run against the composed router.
- Nothing checks a product's crate graph after generation. A `baukit check` rule reading
  `cargo metadata` would.

## Consequences

Domain and service tests run without a database. Adding a feature touches up to six crates, and
small products pay that cost too. Products wire their own extra adapters in `compose_api` and get
the security layers whatever they add.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): the template started with the hexagonal crates.
- 2026-11-07, commit `213caa32`: `compose_api` replaced per-binary wiring.
