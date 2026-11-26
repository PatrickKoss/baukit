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
| 0007 | [Baukit is a toolkit of opt-in units](0007-toolkit-of-opt-in-units.md) | Accepted |
| 0008 | [One release train, registry pins, 0.x support policy, MIT](0008-release-train-and-support-policy.md) | Accepted |
| 0009 | [Generated backends are hexagonal, with compose_api in the bin crate](0009-hexagonal-generated-backend.md) | Accepted |
| 0010 | [Scaffolding has one lifecycle: manifest, re-render, doctor, golden trees, fixture gate](0010-scaffolding-lifecycle.md) | Accepted |
| 0011 | [API and operations listen on separate listeners with a traffic gate](0011-separate-api-and-ops-listeners.md) | Accepted |
| 0012 | [HTTP APIs share one wire contract](0012-http-wire-contract.md) | Accepted |
| 0013 | [Code rules are enforced by baukit check and inherited by products](0013-code-rules-enforced-by-baukit-check.md) | Accepted |
| 0014 | [One gate runs every check in lanes](0014-one-quality-gate.md) | Accepted |
| 0015 | [Every route is rate-limited and tenant SQL is owner-scoped](0015-request-admission.md) | Accepted |
| 0016 | [All outbound HTTP goes through baukit-egress with DNS pinning](0016-outbound-http-through-egress.md) | Accepted |
| 0017 | [Identity providers and bearer verification; Keycloak for development only](0017-identity-providers-and-bearer-verification.md) | Accepted |
| 0018 | [Secrets at rest are credential vault v2 records](0018-credential-vault-records.md) | Accepted |
| 0019 | [MCP is remote only, as an OAuth protected resource](0019-remote-mcp-only.md) | Accepted |
| 0020 | [Durable jobs use a PostgreSQL outbox with attempt-bound leases](0020-durable-jobs-outbox-and-leases.md) | Accepted |
| 0021 | [Job ownership keys for erasure](0021-job-ownership-keys-for-erasure.md) | Proposed |
| 0022 | [Client local data uses per-profile partitions and readiness states](0022-client-local-data-partitions.md) | Accepted |
| 0023 | [Sync uses per-owner revisions, a bounded pull window, and HLC for ordering only](0023-sync-revisions-and-pull-window.md) | Accepted |
| 0024 | [Suite linking, HMAC-signed delivery, Ed25519 entitlement tickets](0024-suite-linking-and-signed-delivery.md) | Accepted |
| 0025 | [Erasure and data-subject requests](0025-erasure-and-data-subject-requests.md) | Accepted |
| 0026 | [Schema migrations ship as reference SQL, expand and contract, never on startup](0026-reference-sql-migrations.md) | Accepted |
| 0027 | [Verification uses one tool per property class](0027-verification-strategy.md) | Accepted |
| 0028 | [TypeScript packages are ESM only with a core and adapter split](0028-typescript-package-format.md) | Accepted |
| 0029 | [Observability contract: resource attributes and fixed metric names](0029-observability-contract.md) | Accepted |
| 0030 | [Deployment platform: Flux bases, local k3d harness, per-product namespaces](0030-deployment-platform.md) | Accepted |
| 0031 | [OpenAPI mirrors](0031-openapi-mirrors.md) | Deferred |
| 0032 | [Push delivery: provider-neutral port, install secrets, delivery claims](0032-push-delivery.md) | Accepted |
| 0033 | [Signed media grants shared between Rust and nginx njs](0033-signed-media-grants.md) | Accepted |
