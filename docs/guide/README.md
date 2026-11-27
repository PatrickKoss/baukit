# baukit guide

The guide shows how to do one task with baukit at a time. Each page links the crate READMEs, contracts and ADRs that hold the detail. Read it in this order the first time.

- [Getting started](getting-started.md): install the CLI, scaffold a product and run its checks.
- [Scaffolding](scaffolding.md): every `baukit new` flag and what it renders.
- [CLI](cli.md): `new`, `doctor`, `generate` and `check`.
- [Manifest](manifest.md): the `baukit.toml` reference.
- [Backend](backend.md): runtime, config, HTTP, the error envelope, the ops listener and OpenAPI drift.
- [Authentication](auth.md): OIDC, Clerk and WorkOS, tokens, browser sessions and a local Keycloak.
- [Data and sync](data-and-sync.md): sync tables, pull windows, push batches and the client sync loop.
- [Jobs](jobs.md): the outbox, the worker, leases and recurring jobs.
- [Remote MCP](mcp.md): the OAuth flow, tool ports, client setup and personal access tokens.
- [Suite and platform](suite-and-platform.md): suites, hubs, signed events, entitlements and mobile purchases.
- [Integrations](integrations.md): connectors, verified webhooks, OAuth state and the credential vault.
- [Push and notifications](push-and-notifications.md): device registration, delivery, receipts and local notifications.
- [Privacy](privacy.md): profile erasure, data-subject requests and analytics consent.
- [Outbound HTTP and limits](outbound-and-limits.md): `baukit-egress` and rate limiting.
- [Frontend](frontend.md): tokens, accessibility, navigation, localization, analytics, PWA and CSP.
- [Observability](observability.md): metrics, dashboards, alerts and runbook URLs.
- [Deploying](deploying.md): images, release workflows, GitOps and the Helm chart.
- [Local platform](local-platform.md): run the platform on k3d.
- [Quality gates](quality-gates.md): standard and strict gates, lanes and budgets.
- [Security](security.md): what the gates and inherited suites check, and what a product still owns.
- [Upgrading](upgrading.md): move a product to a new baukit release.
- [Agent skills](agent-skills.md): install the skills and what each one does.

## Rust crates

| Crate | What it does | Guide |
|---|---|---|
| [`baukit-runtime`](../../rust/crates/baukit-runtime/README.md) | Process lifecycle, graceful drain, supervised background tasks and listener composition. | [Backend](backend.md) |
| [`baukit-config`](../../rust/crates/baukit-config/README.md) | Layered file and environment config with validation and secret wrappers that refuse to print. | [Backend](backend.md) |
| [`baukit-http`](../../rust/crates/baukit-http/README.md) | Axum middleware, the error envelope, upstream retry classification, CORS and the default cache policy. | [Backend](backend.md) |
| [`baukit-ops`](../../rust/crates/baukit-ops/README.md) | Separate liveness, readiness, metrics and build-info endpoints on a private listener. | [Backend](backend.md) |
| [`baukit-telemetry`](../../rust/crates/baukit-telemetry/README.md) | Structured logs, OpenTelemetry traces and Prometheus metrics from one builder. | [Observability](observability.md) |
| [`baukit-openapi`](../../rust/crates/baukit-openapi/README.md) | Utoipa metadata, deterministic schema output and drift checks. | [Backend](backend.md) |
| [`baukit-core`](../../rust/crates/baukit-core/README.md) | Dependency-light vocabulary shared by the other crates, plus optional keyset pagination. | [Backend](backend.md) |
| [`baukit-auth`](../../rust/crates/baukit-auth/README.md) | OIDC, Clerk and WorkOS verification, personal access tokens and Axum principal extraction. | [Authentication](auth.md) |
| [`baukit-sync`](../../rust/crates/baukit-sync/README.md) | Per-owner revision allocation, pull windows and tombstone purges for offline clients. | [Data and sync](data-and-sync.md) |
| [`baukit-jobs`](../../rust/crates/baukit-jobs/README.md) | Durable PostgreSQL outbox and supervised workers. | [Jobs](jobs.md) |
| [`baukit-mcp`](../../rust/crates/baukit-mcp/README.md) | OAuth-protected Streamable HTTP MCP with scoped tool ports, resource metadata, body limits and quotas. | [Remote MCP](mcp.md) |
| [`baukit-suite`](../../rust/crates/baukit-suite/README.md) | Peer linking, signed event delivery and transactional ingestion. | [Suite and platform](suite-and-platform.md) |
| [`baukit-events`](../../rust/crates/baukit-events/README.md) | Versioned event envelope with stable validation codes. | [Suite and platform](suite-and-platform.md) |
| [`baukit-integrations`](../../rust/crates/baukit-integrations/README.md) | Connector contract for cursor-paged imports, verified webhooks and connection health. | [Integrations](integrations.md) |
| [`baukit-credential-vault`](../../rust/crates/baukit-credential-vault/README.md) | Versioned AES-256-GCM encryption behind a storage-neutral port. | [Integrations](integrations.md) |
| [`baukit-push`](../../rust/crates/baukit-push/README.md) | Provider-neutral push delivery with an Expo adapter, a device registry and daily delivery claims. | [Push and notifications](push-and-notifications.md) |
| [`baukit-erasure`](../../rust/crates/baukit-erasure/README.md) | Transactional profile erasure, keyed receipts and fences, and durable identity account deletion. | [Privacy](privacy.md) |
| [`baukit-data-subject`](../../rust/crates/baukit-data-subject/README.md) | Data-subject wire types, identifier normalization and Ed25519 DSSE receipts. | [Privacy](privacy.md) |
| [`baukit-egress`](../../rust/crates/baukit-egress/README.md) | Outbound HTTP client that pins DNS answers to allowed addresses. | [Outbound HTTP and limits](outbound-and-limits.md) |
| [`baukit-ratelimit`](../../rust/crates/baukit-ratelimit/README.md) | Redis token buckets keyed by identity or client IP. | [Outbound HTTP and limits](outbound-and-limits.md) |
| [`baukit-test`](../../rust/crates/baukit-test/README.md) | Docker PostgreSQL, Redis and Keycloak fixtures, a mock OIDC issuer and conformance suites. | [Backend](backend.md) |

## TypeScript packages

| Package | What it does | Guide |
|---|---|---|
| [`@baukit/api-runtime`](../../typescript/packages/api-runtime/README.md) | Auth and request-ID headers, trace propagation, normalized errors and safe retries. | [Backend](backend.md) |
| [`@baukit/auth-web`](../../typescript/packages/auth-web/README.md) | Browser sign-in through the browser-session BFF, plus Clerk and WorkOS adapters. | [Authentication](auth.md) |
| [`@baukit/auth-native`](../../typescript/packages/auth-native/README.md) | Native OIDC PKCE, Clerk and WorkOS adapters with refresh rotation. | [Authentication](auth.md) |
| [`@baukit/auth-node`](../../typescript/packages/auth-node/README.md) | OIDC device authorization with S256 PKCE, bounded polling, refresh rotation and a locked profile cache. | [Authentication](auth.md) |
| [`@baukit/data-contracts`](../../typescript/packages/data-contracts/README.md) | Local storage contracts plus conformance suites you run against your adapter. | [Data and sync](data-and-sync.md) |
| [`@baukit/data-contracts-dexie`](../../typescript/packages/data-contracts-dexie/README.md) | IndexedDB adapter for the data contracts, checked in real browsers. | [Data and sync](data-and-sync.md) |
| [`@baukit/data-contracts-expo-sqlite`](../../typescript/packages/data-contracts-expo-sqlite/README.md) | Expo SQLite adapter for the data contracts, checked on an Android device. | [Data and sync](data-and-sync.md) |
| [`@baukit/sync-client`](../../typescript/packages/sync-client/README.md) | Sync scheduling, transport, status store and push-batch ordering. | [Data and sync](data-and-sync.md) |
| [`@baukit/suite-client`](../../typescript/packages/suite-client/README.md) | Suite linking, authorization and connected-app state for web and native clients. | [Suite and platform](suite-and-platform.md) |
| [`@baukit/events`](../../typescript/packages/events/README.md) | Zod schemas for the envelope that `baukit-events` speaks. | [Suite and platform](suite-and-platform.md) |
| [`@baukit/purchases-core`](../../typescript/packages/purchases-core/README.md) | Store-neutral purchase flow that gates features on the backend-verified entitlement. | [Suite and platform](suite-and-platform.md) |
| [`@baukit/purchases-revenuecat`](../../typescript/packages/purchases-revenuecat/README.md) | RevenueCat adapter for `@baukit/purchases-core`. | [Suite and platform](suite-and-platform.md) |
| [`@baukit/integrations-client`](../../typescript/packages/integrations-client/README.md) | Connection health, OAuth session coordination and a provider registry. | [Integrations](integrations.md) |
| [`@baukit/notifications-core`](../../typescript/packages/notifications-core/README.md) | Local notification planning across DST and time zones. | [Push and notifications](push-and-notifications.md) |
| [`@baukit/notifications-expo`](../../typescript/packages/notifications-expo/README.md) | Expo adapter that replaces only the notification requests one namespace owns. | [Push and notifications](push-and-notifications.md) |
| [`@baukit/analytics-core`](../../typescript/packages/analytics-core/README.md) | Typed analytics events with consent and privacy controls. | [Frontend](frontend.md) |
| [`@baukit/analytics-posthog-web`](../../typescript/packages/analytics-posthog-web/README.md) | PostHog transport for `@baukit/analytics-core` on the web. | [Frontend](frontend.md) |
| [`@baukit/analytics-posthog-native`](../../typescript/packages/analytics-posthog-native/README.md) | PostHog transport for `@baukit/analytics-core` in React Native. | [Frontend](frontend.md) |
| [`@baukit/ui-tokens`](../../typescript/packages/ui-tokens/README.md) | Design-token schema, contrast checks, CSS and React Native compilers and a `no-raw-color` eslint rule. | [Frontend](frontend.md) |
| [`@baukit/a11y-core`](../../typescript/packages/a11y-core/README.md) | Focus traps, inert backgrounds, announcements and reduced motion on web and native. | [Frontend](frontend.md) |
| [`@baukit/navigation`](../../typescript/packages/navigation/README.md) | Collapsible navigation for React DOM and React Native with product-owned routes and tokens. | [Frontend](frontend.md) |
| [`@baukit/localization-core`](../../typescript/packages/localization-core/README.md) | Locale resolution, formatting policy and time-zone-safe civil-date math. | [Frontend](frontend.md) |
| [`@baukit/preferences-core`](../../typescript/packages/preferences-core/README.md) | Identity-guarded preference controller. | [Frontend](frontend.md) |
| [`@baukit/pwa-web`](../../typescript/packages/pwa-web/README.md) | Request classification and cache strategies for a service worker you own. | [Frontend](frontend.md) |
| [`@baukit/csp`](../../typescript/packages/csp/README.md) | Content Security Policy builder and security headers for web builds. | [Frontend](frontend.md) |
| [`@baukit/type-guards`](../../typescript/packages/type-guards/README.md) | Runtime type guards for parsing unknown JSON. | [Frontend](frontend.md) |
| [`@baukit/eslint-config`](../../typescript/packages/eslint-config/README.md) | Shared ESLint flat configs for baukit packages and products. | [Frontend](frontend.md) |
