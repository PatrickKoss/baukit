# Architecture decision records

An ADR records why baukit is built the way it is today and what that choice replaced. Each one
describes the code at the commit it was last amended, names the check that fails when someone breaks
the decision, and keeps earlier decisions in its History section.

## Status vocabulary

The `## Status` section holds one line. It starts with one of these values, and a date may follow,
for example "Accepted, 2026-11-27.":

- `Proposed` means the decision is written down but the code does not follow it yet.
- `Accepted` means the code follows the decision.
- `Amended YYYY-MM-DD` means the decision was changed in place on that date. History says what changed.
- `Superseded by NNNN` means another ADR replaced the decision.
- `Deferred` means the choice was considered and postponed. The ADR says what would reopen it.

## Template

Copy this into `docs/adr/NNNN-<kebab-title>.md`. Keep the headings in this order and the file under
120 lines.

```markdown
# ADR NNNN: <decision as a sentence>

## Status
Accepted, YYYY-MM-DD.

## Context
What forced a choice. Two paragraphs at most.

## Decision
What baukit does today, in present tense. Name types, crates, defaults.

## Enforced by
The test, lint, gate step or type that fails when the decision is broken. "Nothing" is an allowed answer and a signal.

## Consequences
What products get and what they give up.

## History
Earlier decisions this one replaced, with dates and commits.
```

An `Enforced by` section that says "Nothing." marks a decision that can drift without anyone
noticing. It is a candidate for a new check or for the `Proposed` status.

## Index

| Number | Title | Status |
|---|---|---|
| 0001 | [Product experience packages hold behavior, not design](0001-product-experience-package-boundaries.md) | Amended 2026-11-27 |
| 0002 | [Integration connectors share one contract crate](0002-integration-connector-contract.md) | Amended 2026-11-27 |
| 0003 | [Suite events share one envelope](0003-event-envelope.md) | Amended 2026-11-27 |
| 0004 | [Integration clients share one dependency-free contract](0004-integration-client-contract.md) | Accepted |
| 0005 | [Products share one application navigation package](0005-shared-application-navigation.md) | Accepted |
| 0006 | [The browser never holds a refresh token](0006-browser-token-handling.md) | Amended 2026-11-27 |
| 0007 | [Baukit is a toolkit of opt-in units with contract cores and feature-gated adapters](0007-toolkit-of-opt-in-units.md) | Accepted |
| 0008 | [All units release together on one train, products pin registry versions, and the license is MIT](0008-release-train-and-support-policy.md) | Accepted |
| 0009 | [Generated backends are hexagonal crates, wired in one compose_api function](0009-hexagonal-generated-backend.md) | Accepted |
| 0010 | [The CLI scaffolds products from a manifest, re-renders without overwriting, and is held to golden trees](0010-scaffolding-lifecycle.md) | Accepted |
| 0011 | [API and operations endpoints use separate listeners, with a traffic gate for drain](0011-separate-api-and-ops-listeners.md) | Accepted |
| 0012 | [HTTP APIs share one error envelope, camelCase names, committed OpenAPI and idempotency keys](0012-http-wire-contract.md) | Accepted |
| 0013 | [Code rules live in baukit check, and products inherit the same rules](0013-code-rules-enforced-by-baukit-check.md) | Accepted |
| 0014 | [One gate script runs every CI check locally, in parallel lanes](0014-one-quality-gate.md) | Accepted |
| 0015 | [Every route sits inside the rate limiter and tenant SQL puts the owner in the WHERE](0015-request-admission.md) | Accepted |
| 0016 | [All outbound HTTP goes through baukit-egress with DNS pinning](0016-outbound-http-through-egress.md) | Accepted |
| 0017 | [Bearer tokens are verified by baukit-auth per provider profile, and Keycloak is for development only](0017-identity-providers-and-bearer-verification.md) | Accepted |
| 0018 | [Stored secrets are sealed as credential vault v2 records](0018-credential-vault-records.md) | Accepted |
| 0019 | [MCP servers are remote only, as OAuth protected resources](0019-remote-mcp-only.md) | Accepted |
| 0020 | [Durable jobs run from a PostgreSQL outbox with attempt-bound leases](0020-durable-jobs-outbox-and-leases.md) | Accepted |
| 0021 | [Jobs carry an optional owner key so erasure can cancel and delete them](0021-job-ownership-keys-for-erasure.md) | Proposed |
| 0022 | [Client local data lives in one partition per profile with explicit readiness states](0022-client-local-data-partitions.md) | Accepted |
| 0023 | [Sync orders rows by per-owner revisions in a bounded pull window, and HLC orders writes only](0023-sync-revisions-and-pull-window.md) | Accepted |
| 0024 | [Suite apps link with PKCE, deliver HMAC-signed events and verify Ed25519 entitlement tickets](0024-suite-linking-and-signed-delivery.md) | Accepted |
| 0025 | [Erasure commits in one transaction behind a subject fence, and data subject requests return signed receipts](0025-erasure-and-data-subject-requests.md) | Accepted |
| 0026 | [Crates ship reference SQL that products copy and apply as expand and contract steps, never on API startup](0026-reference-sql-migrations.md) | Accepted |
| 0027 | [Each property class gets one verification tool, and every tool runs as a gate step](0027-verification-strategy.md) | Accepted |
| 0028 | [TypeScript packages ship ES modules, pass a packed-export test, and keep vendor SDKs in adapter packages](0028-typescript-package-format.md) | Accepted |
| 0029 | [Every baukit backend emits the same resource attributes and fixed metric names, and shipped dashboards may query only those names](0029-observability-contract.md) | Accepted |
| 0030 | [The platform is a menu of Flux Kustomize bases for single-node k3s, tested on a local k3d cluster, with one tenant namespace per product environment](0030-deployment-platform.md) | Accepted |
| 0031 | [Raw OpenAPI mirrors wait for a second consumer and would be a separate manifest list from declaration consumers](0031-openapi-mirrors.md) | Deferred |
| 0032 | [Push goes through a provider-neutral port, device tokens move between owners only with an install secret, and scheduled pushes take a delivery claim](0032-push-delivery.md) | Accepted |
| 0033 | [The backend signs short-lived media grants in Rust and nginx verifies them in njs, both held to one vector file](0033-signed-media-grants.md) | Accepted |
