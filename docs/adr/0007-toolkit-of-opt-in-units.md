# ADR 0007: Baukit is a toolkit of opt-in units with contract cores and feature-gated adapters

## Status
Accepted, 2026-11-27.

## Context

Baukit serves several independent products. A framework would own their main loop, their schema
and their release pace. Each product would then pay for every feature any other product needed,
and a change for one product would break the rest.

Shared code also tends to grow from one product's needs. Code that only one product uses has not
shown which parts are general, so moving it into baukit fixes that product's choices for everyone.

## Decision

- Each Rust crate under `rust/crates/` and each package under `typescript/packages/` is a unit a
  product adds on its own. Products depend on the units they use and on nothing else. There is no
  root crate or package that pulls them all in.
- A unit's core is a contract: types, traits as ports, validation and pure logic. Adapters that
  need a database, an HTTP stack or a framework sit behind Cargo features or separate packages.
  Examples:
  - `baukit-integrations`: contract by default, `oauth-state`, `postgres` and `axum` add adapters.
  - `baukit-suite`: no default features, with `postgres`, `http`, `jobs`, `runtime`, `delivery` and
    `platform` features.
  - `baukit-auth`: `sqlx-postgres` and `browser-session`. `baukit-sync`, `baukit-ops` and
    `baukit-push`: `sqlx-postgres`.
  - TypeScript splits a core from its adapters by package, for example `@baukit/data-contracts`
    with `-dexie` and `-expo-sqlite`, or `@baukit/analytics-core` with the PostHog packages.
- A few crates turn on an adapter by default because nearly every user wants it:
  `baukit-data-subject` (`server`), `baukit-erasure` (`keycloak`, `api-providers`) and `baukit-test`
  (`sqlx-postgres`).
- Third-party Rust dependencies use caret requirements. An exact pin needs a documented semver
  incompatibility and an entry in `EXACT_PIN_ALLOWLIST` in
  `scripts/test_rust_dependency_requirements.py`. Internal `baukit-*` requirements stay exact,
  because the units release together ([ADR 0008](0008-release-train-and-support-policy.md)).
- Code moves into baukit when a second product needs it. Until then it stays in the product, even if
  it looks general. Product names, product schemas and product copy never enter baukit.

## Enforced by

- Gate step `suite-features` runs `scripts/check-suite-features.sh`. It fails when
  `baukit-suite --no-default-features` pulls in sqlx, tokio, reqwest, axum, utoipa or the runtime
  baukit crates, and it builds each feature alone.
- `scripts/test_rust_dependency_requirements.py`:
  `test_repository_has_only_justified_exact_third_party_pins` fails on an unlisted exact pin.
- Gate step `package-dependencies` runs `scripts/check-package-dependencies.py`, which fails on a
  `workspace:`, `link:` or `file:` specifier in a published npm archive.
- Nothing checks the other crates' default feature sets or the second-consumer rule. The suite
  check could run for every crate with a contract core.

## Consequences

A product that needs one contract compiles only that contract. It keeps its own schema, routes and
UI, and it can replace any adapter with its own. In return, products write their own wiring, and
shared code arrives later than it would in a framework. Products accept duplication until
a second consumer proves the shape.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): the repository started as a set of crates and
  packages with this layout. The rule lived in `CLAUDE.md` and in each early ADR's list of what
  stays product side.
- 2026-11-27: written down as an ADR.
