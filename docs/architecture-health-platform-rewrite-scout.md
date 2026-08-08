# Wave G0 — architecture-health-platform read-only scout

Scout date: 2026-08-09. Repository remained clean; no files were created or modified. Tests/builds were not run because they can write caches and artifacts, so quality findings below are based on static inspection.

## Executive recommendation

Rewrite this as a **web-only, OIDC-authenticated Rust application with a mandatory worker**:

```text
baukit new architecture-health-platform --backend --web --auth oidc
```

Required capabilities:

| Capability | Decision |
|---|---|
| Backend | Yes |
| Web | Yes — Vite/React/TanStack, replacing Next.js |
| Mobile | No |
| Auth | OIDC/Keycloak |
| Worker | Yes — analysis cannot run safely in API processes |
| Offline | No |
| SSR | No |
| Object storage | Not for MVP |
| Vector database | Not for MVP |

The existing repository is a broad, rapidly built TypeScript prototype: approximately 139k tracked TS/TSX lines, 88 Prisma models, 236 tRPC procedures, 31 BullMQ queues, 52 page routes, and 136 active OpenSpec specifications. Its valuable center is the static-analysis pipeline, finding lifecycle, health history, quality gates, and CI/PR workflow. The enterprise administration, billing, compliance, reporting, LLM auto-fix, vector search, and infrastructure breadth should not define the first rewrite.

## 1. Inventory

### Repository shape and size

There are no Rust crates or Cargo workspace today. It is a pnpm 10/Turborepo TypeScript monorepo:

```text
apps/
  web/                 Next.js 16 UI, tRPC server and REST/API routes
  workers/             BullMQ processors and Express Bull Board
packages/
  ai-fix-engine/
  analysis-engine/
  auth/
  billing/
  ci-comment-bridge/
  compliance/
  config/
  database/
  integrations/
  license/
  llm/
  notifications/
  patterns/
  quality-gates/
  report-templates/
  storage/
  telemetry-collector/
  types/
tools/
  cli/
  ebpf-agent/          Go
  github-action/
  gitlab-ci/
tests/
  integration/
  e2e/
deploy/
  docker/
  helm/
  terraform/aws/
  terraform/gcp/
```

Approximate tracked size:

| Area | Files | Relevant lines |
|---|---:|---:|
| `apps/web` | 230 | 49k |
| `apps/workers` | 68 | 12.6k |
| `packages` | 575 | 74.5k |
| `tools` | 35 | 2.7k |
| `tests` | 35 | 7.1k |
| `deploy` | 83 | 7.2k |
| `openspec` | 430 | 33.6k |
| TS/TSX total | 750 | 139,357 |
| Prisma schema | 1 | 2,122 |
| Terraform | 38 | 3,729 |

There are 1,499 tracked files and only 44 commits; most product breadth was added between 2026-02-08 and 2026-02-20.

### Product features represented

The code contains or models all of the following:

- Repository connection, scheduled scans, manual analyses, push analyses, and PR-delta analyses.
- Static dependency graphs, topology, entity extraction, network-boundary detection, domain clustering, architecture drift, historical churn, and refactoring recommendations.
- Findings, persistent remediation items, status history, assignments, SLA metrics, external issue links, and automatic resolution/reopening by fingerprint.
- Versioned declarative rules, custom rules, shared-rule catalog, rule templates, per-repository rulesets, and configurable thresholds/severities.
- Quality gates, gate templates, evaluation history, false positives, analytics, badges, CLI exit codes, GitHub Action, and GitLab CI template.
- Composite health, technical-debt, reliability, API, security, observability, database, test-quality, technology, and traceability scores.
- Repository/project/team/organization/tenant hierarchy, fine-grained RBAC, enterprise IdP configuration, and role mappings.
- ADR authoring, reviews, lifecycle, change links, impact measurement, auto-suggestions, and supersession detection.
- Git/doc/issue-tracker integrations, document ingestion, chunking, vector indexing, and semantic search.
- Runtime telemetry collection, service topology, static/runtime drift, and a custom Go eBPF agent.
- LLM finding enrichment, code review, consistency checks, fix generation/application, and feedback.
- PR risk, contextual recommendations, review-comment lifecycle, and team-process recommendations.
- Technology inventory, lifecycle policies, radar, fragmentation, and migration tracking.
- Compliance frameworks, posture, snapshots, overrides, evidence exports, and dashboards.
- Audit hash chains, S3 exports, API keys, outgoing webhooks, notification channels, digests, and escalation.
- CSV/PDF reports, schedules, comparisons, benchmark cohorts, billing quotas, licensing, air-gap mode, and self-hosted usage telemetry.

Several UI routes are placeholders rather than connected product surfaces, notably repository lists/details, integration documents, ADR overview, rules settings, and analysis diff pages.

### Domain and data model

`packages/database/prisma/schema.prisma` defines **88 models and 56 enums**, backed by PostgreSQL. They group as follows.

Identity, tenancy and organization:

- `User`, `Tenant`, `TenantDomain`
- `Organization`, `OrganizationSettings`, `UsageCounter`, `OrganizationMember`
- `Team`, `TeamMember`
- `Project`, `ProjectTeam`, `ProjectRepository`
- `RoleAssignment`, `IdpConfig`, `IdpRoleMapping`

Repository and analysis:

- `Repository`, `Analysis`, `Finding`, `Dependency`, `Metric`
- `HealthScore`, `RulesetConfig`, `TechDebtScore`
- `QualityGateConfig`, `QualityGateEvaluation`, `QualityGateFalsePositive`
- `ArchitecturePattern`

Finding lifecycle and remediation:

- `RemediationItem`, `FindingStatusHistory`, `ExternalIssueLink`
- `RefactoringRec`, `Fix`, `FixFeedback`

Static/runtime topology and history:

- `Entity`, `EntityRelation`, `NetworkDep`
- `Domain`, `DomainMembership`
- `Service`, `ServiceDependency`, `ServiceDependencySnapshot`
- `ArchitectureDrift`
- `FileChurnSnapshot`, `BugHotspotSnapshot`, `ArchitectureSnapshot`
- `GitHistoryCache`, `PRRiskScore`, `PRComment`

Integrations, documents and AI:

- `Integration`, `Document`, `DocumentChunk`, `LLMGeneration`

ADR governance:

- `ADR`, `ADRReview`, `ADRReviewConfig`, `ADRChangeLink`
- `ADRImpactSnapshot`, `ADRSuggestion`
- `ADRSupersessionFlag`, `ADRSupersessionExclusion`
- `ADRTemplateRecord`

Reports, audit and machine API:

- `AuditLog`, `AuditS3ExportConfig`
- `Report`, `ReportSchedule`
- `ApiKey`, `WebhookEndpoint`, `WebhookDelivery`

Compliance and notifications:

- `ComplianceSnapshot`, `OrganizationFramework`, `ComplianceOverride`
- `AlertRule`, `NotificationChannel`, `NotificationPreference`
- `NotificationDeliveryLog`, `EscalationChain`, `EscalationStep`

Custom rules, technology and benchmarking:

- `CustomRule`, `CustomRuleVersion`, `SharedRule`
- `DetectedTechnology`, `TechRadarEntry`, `TechLifecyclePolicy`
- `BenchmarkSnapshot`, `BenchmarkPercentile`

Spec traceability:

- `SpecSource`, `SpecRequirement`, `TraceabilityLink`

The central current relationship is:

```text
Tenant
  └─ Organization
      ├─ Members / Teams / Projects
      ├─ Integrations
      └─ Repositories
          ├─ Analyses
          │   ├─ Findings
          │   ├─ Dependencies / Metrics
          │   ├─ Entities / Domains / NetworkDeps
          │   └─ TraceabilityLinks
          ├─ HealthScores
          ├─ RemediationItems
          └─ Ruleset / QualityGate configuration
```

Only `0001_initial/migration.sql` exists for the entire 88-model schema; normal development instructions use `prisma db push`. The rewrite should use small, ordered SQLx migrations from its first product model.

### Analysis pipeline

The core is `packages/analysis-engine/src/analyze.ts`, with supporting modules under:

- `scanner.ts`
- `parsers/`
- `extractors/`
- `graph.ts`
- `detectors/`
- `scorers/`
- `rules/`
- `clustering/`
- `tech/`
- `linkers/`
- `team-process/`

Current full-analysis flow:

1. Scan configured source extensions, honoring standard ignore paths and a 10,000-line file cap.
2. Initialize `web-tree-sitter` and parse supported files.
3. Extract imports, dependencies, entities, entity relations, functions, and possible network calls.
4. Build an adjacency graph and run Tarjan-style strongly connected component/cycle analysis.
5. Run direct detectors for circular dependencies, god classes, coupling, layer violations, complexity, and missing abstractions.
6. Match architectural patterns.
7. Optionally register and execute standard/custom rules.
8. Detect dependency debt and technologies from manifests.
9. Link spec requirements to code.
10. Calculate component, technical-debt, and overall health scores.
11. Run Louvain-style domain clustering.
12. Generate refactoring recommendations and return the complete analysis result.

The scanner recognizes 13 language families: TypeScript, JavaScript, Python, Java, Go, Rust, Ruby, PHP, C#, Kotlin, and Swift extensions. Actual AST parsers exist only for TypeScript/JavaScript, Python, Java, and Go. Technology manifest parsing covers npm, Go, Python, Java, Ruby, Rust, and .NET.

The standard rules directory contains **77 YAML descriptors** across:

- API: 17
- Code quality: 12
- Database: 4
- Metrics: 4
- Observability: 5
- Performance: 4
- Reliability: 14
- Security: 5
- Structural: 9
- Technical debt: 3

Other data catalogs include five pattern definitions, four ADR templates, five quality-gate templates, and four compliance frameworks (`hipaa`, `iso27001`, `pci-dss`, `soc2`).

The main analysis worker clones a repository into a temporary directory, loads rules/custom rules/history/specs, executes analysis, persists results, reconciles remediation items, evaluates gates, emits notifications and outgoing webhooks, queues benchmark/LLM work, and deletes the clone. A separate PR path clones a pull-request ref, calculates changed files and finding deltas, estimates risk, and posts provider review comments.

### Integrations and external services

Git provider ports and adapters in `packages/integrations`:

- GitHub
- GitLab
- Bitbucket
- Azure DevOps

Documentation sources:

- Confluence
- Markdown repository
- Docusaurus
- Astro

Issue trackers:

- Jira
- Linear
- GitHub Issues

Authentication:

- Clerk
- Keycloak
- Local development stub
- Enterprise OIDC/SAML
- GitHub OAuth/App connection flow

LLM providers:

- Anthropic
- OpenAI chat and embeddings
- OpenRouter

Notification delivery:

- Slack
- Microsoft Teams
- Resend email
- Generic webhook
- PagerDuty

Stateful/external infrastructure:

- PostgreSQL via Prisma
- Redis/BullMQ
- Qdrant
- S3-compatible storage/MinIO; GCS is used through its S3-compatible credentials in deployment
- Git/GitHub APIs
- Playwright/Chromium for PDF rendering
- OTLP/gRPC telemetry endpoints
- Optional license-validation and self-hosted telemetry endpoints

Credential encryption uses an application-level AES-GCM helper and encrypted JSON stored on `Integration`/`NotificationChannel`. This should be redesigned around product ports and deployment secrets rather than copied.

### API surface

The current web application is also a second backend runtime.

#### tRPC

`apps/web/src/server/routers/_app.ts` composes **40 namespaces and approximately 236 procedures**:

- `repository`, `analysis`, `findings`, `health`
- `project`, `team`, `organization`, `tenant`
- `topology`, `runtime`, `techDebt`, `techRadar`
- `adr`, `remediation`, `fix`, `prRisk`
- `integration`, `document`, `search`
- `ruleset`, `customRule`, `qualityGate`
- `specSource`, `traceability`
- `compliance`, `benchmarks`, `executive`, `reports`
- `alertRules`, `notificationChannels`, `notificationPreferences`, `notificationHistory`, `escalationChains`
- `apiKeys`, `webhooks`, `audit`, `billing`
- `idpConfig`, `roleMapping`, `llmSettings`

Representative operations include:

- Repository `list`, `getById`, `connect`, `disconnect`, `updateSettings`
- Analysis `getLatest`, `getById`, `listForRepository`, `getDiff`
- Findings `list`, `getById`, `getTopByRepository`
- Topology `getGraph`, `getDomains`, `getNetworkBoundaries`, `getRecommendations`
- Rules `listRules`, `getConfig`, `upsertConfig`; custom-rule version/test/import/publish operations
- Quality-gate configuration, evaluation, false-positive and analytics operations
- Remediation status/assignment/bulk mutations and SLA reporting
- ADR review, suggestion, impact and supersession operations
- Traceability matrix, coverage and manual-link operations

#### Documented REST API

The OpenAPI 3.1 registry documents 11 operations:

```text
GET    /api/v1/repositories
POST   /api/v1/repositories
GET    /api/v1/repositories/{id}
DELETE /api/v1/repositories/{id}
GET    /api/v1/repositories/{id}/analyses
POST   /api/v1/repositories/{id}/analyses
GET    /api/v1/analyses/{id}
GET    /api/v1/repositories/{id}/health
GET    /api/v1/repositories/{id}/health/history
GET    /api/v1/repositories/{id}/findings
GET    /api/v1/findings/{id}
```

Additional HTTP routes include:

- `GET /api/v1/openapi.json`
- `GET /api/v1/repositories/{id}/benchmarks`
- `POST /api/webhooks/{provider}`
- `POST /api/webhooks/github`
- OIDC, Keycloak, SAML and GitHub auth callbacks
- `/api/trpc/{trpc}`

REST authentication uses bearer API keys with plan-dependent scopes/rate limits. tRPC uses browser authentication and product RBAC. The rewrite should expose one Rust/OpenAPI contract instead of retaining this split.

### Frontend

There is one frontend: `apps/web`, using Next.js 16, React 19, Tailwind 3, Radix primitives, TanStack Query, tRPC, Recharts, Monaco, ELK graph layout, and `diff2html`.

It has 52 page routes covering:

- Portfolio/executive dashboard
- Projects, repositories, analyses and findings
- Dependency/topology visualization
- ADRs and ADR governance
- Quality gates and custom rules
- Integrations and documents
- Compliance
- Fixes/remediation
- Teams and tenant settings
- Notifications, audit and role mapping
- Technology radar
- Search

Notable visualization components include `dependency-graph`, `topology-graph`, `annotated-diff-view`, `health-score-circle`, `trend-chart`, `component-scores-breakdown`, `module-metrics-chart`, `file-churn-heatmap`, and benchmark charts.

There is no mobile client, no mobile-specific workflow, and no credible need for one. The dominant interactions—graphs, source locations, diffs, rule editing, dashboards, and repository administration—are desktop/web tasks.

### Workers and jobs

`apps/workers/src/queues.ts` creates **31 BullMQ queues**:

```text
analysis
webhooks
pr-analysis
telemetry
doc-sync
vector-indexing
history-collection
llm-enrichment
remediation-backfill
issue-tracker-sync
compliance
report-generation
report-cleanup
scheduled-reports
api-webhook-delivery
audit-log
audit-retention
audit-s3-export
notification-events
notification-delivery
notification-digests
notification-escalation
adr-impact
adr-lifecycle
self-hosted-telemetry
benchmark-snapshots
benchmark-aggregation
benchmark-retention
fix-generation
fix-application
spec-sync
```

There are 33 worker source files and approximately 25 processors instantiated by `apps/workers/src/index.ts`. Scheduled/repeat work includes:

- Daily stale-repository scans
- Telemetry cleanup
- Monthly compliance snapshots
- Audit retention and S3 export
- ADR impact snapshots
- Daily/weekly benchmark aggregation and retention
- Quarterly benchmark reports
- Daily spec synchronization
- Hourly expired-report cleanup
- Optional daily self-hosted telemetry
- Per-user digest, issue-sync and report schedules

The worker exposes `/health` and Bull Board at `/admin/queues`. Bull Board has no visible authentication boundary in the worker process and should not be retained as a public/admin surface.

### Deployment

Local Compose provides:

- PostgreSQL 16
- Redis 7
- Qdrant
- Keycloak 26
- MinIO
- Gitea

Deployment artifacts include:

- `Dockerfile.web`, `Dockerfile.workers`, and `Dockerfile.migrate`
- Helm chart with web, workers, migration hook, validation hook, ingress, secrets and optional embedded Redis/Qdrant
- GCP Terraform for VPC, GKE, Cloud SQL, Memorystore and GCS
- AWS Terraform for VPC, EKS, RDS, ElastiCache and S3
- Separate Go eBPF-agent Docker/Helm/Kubernetes manifests

The deployment documentation is stale: it describes AWS and Helm as “coming soon” although both are present.

GitHub Actions contains only two manually triggered workflows:

- Container image builds
- Helm lint/package

The push/PR triggers are commented out. There is no active CI workflow for typecheck, lint, unit tests, integration tests, OpenAPI drift or E2E.

### Test suites and quality assessment

Test inventory:

- 211 conventional `.test`/`.spec` source files
- Approximately 2,713 `it()`/`test()` declarations
- 73 analysis-engine test files
- 34 web tests
- 19 worker tests
- 24 integration/E2E tests
- Playwright critical paths for analysis, auth, tenancy, repository connection and traceability
- Testcontainers coverage for PostgreSQL/Redis integrations
- Extensive detector, scorer, ruleset, auth, worker, provider and router unit tests

Strengths:

- Strong fixture-oriented analysis tests.
- Many pure detector/scorer functions.
- Good attention to webhook signatures, auth failures, tenant isolation, worker lifecycle and provider boundaries in tests.
- Declarative rule/pattern/gate content is a useful behavioral reference.
- Broad OpenSpec documentation.

Material quality concerns found statically:

- The project has far more breadth than integration maturity.
- No active code CI or coverage threshold exists.
- `apps/workers/package.json` omits `@ahp/ai-fix-engine`, although `fix-generation-worker.ts` imports it; the worker Dockerfile also does not include that package manifest.
- `fix-generation-worker.ts` selects `Finding.metadata`, but the Prisma `Finding` model has no `metadata` field.
- `analysis-worker.ts` contains a duplicate `where` property in one Prisma call.
- The scanner advertises 13 language families while only five have AST parsers.
- `allClasses` is never populated in `analyze.ts`, making the direct god-class detector ineffective.
- Duplication is hard-coded as zero in maintainability scoring.
- API presence is approximated by “any parsed tree exists.”
- PR risk uses empty graph/function inputs and estimates changed lines as `findings * 10`.
- The generic webhook path converts provider platform IDs into a payload later treated as an internal repository ID.
- GitHub signature verification reconstructs JSON rather than consistently using raw request bytes.
- The separate `/api/webhooks/github` handler verifies and logs but does not enqueue work.
- Helm probes the web container at `/api/health`, but no such route exists.
- Some worker sources/queues are not wired into the main process.
- Multiple dashboard pages contain hard-coded placeholder data.
- Playwright reports and test-result state are committed.
- One initial migration covers the entire schema and development relies on `db push`.

Overall assessment: excellent product exploration and test volume, but prototype-level cohesion and release discipline. Treat specifications, fixture behavior and domain vocabulary as reference material; do not port implementation structure or assume feature parity is desirable.

## 2. Keep / drop / defer

All “keep” items mean reimplement from first principles in the baukit architecture. No existing source should be mechanically migrated.

### Keep for the first rewrite

- OIDC login, subject-to-user mapping, organization membership and basic owner/member authorization.
- Organizations, projects and repository assignment; drop the separate `Tenant` layer initially.
- GitHub repository connection first, including verified webhooks, manual scans, default-branch scans and scheduled scans.
- A provider port that permits GitLab later without contaminating domain/services.
- Dedicated worker process with durable jobs, idempotency, bounded retries and cleanup of temporary clones.
- Static scanner, parser capability matrix, dependency extraction, graph construction and deterministic path normalization.
- High-confidence initial rules:
  - circular dependencies
  - complexity
  - god class
  - coupling
  - layer/domain violations
  - missing abstraction
  - dependency freshness/pinning
  - hard-coded secrets/insecure crypto
  - missing tests
  - missing health/metrics/tracing
- Versioned declarative rule catalog and per-repository enable/threshold/severity configuration.
- Health score concept, but only from measured components; persist scoring-version and evidence.
- Findings, fingerprints, occurrence history, status lifecycle, automatic resolution/reopening and health history.
- Dependency/topology data and repository trend views.
- Quality gates with a small declarative DSL and deterministic CI exit status.
- Rust/OpenAPI API as the only application API; generated TypeScript client for the web app.
- Web dashboard for projects, repositories, health, findings, topology, history and rules.
- Local CLI analysis plus GitHub Action annotations/status.
- PR delta analysis and review comments after full-repository analysis is reliable.
- Baukit telemetry, worker metrics, ops endpoints, conformance tests and shared deployment chart.

### Drop

- Current Next.js/tRPC/Prisma/BullMQ architecture and all dual-API behavior.
- Mechanical migration of any TS implementation.
- Separate `Tenant` and `Organization` hierarchy for the initial product.
- Clerk, local-stub, SAML and provider-specific enterprise auth paths; use standard OIDC.
- In-app enterprise IdP configuration and dynamic role mappings.
- Billing plans, usage quotas, license enforcement, feature gates and upgrade prompts.
- Self-hosted product call-home telemetry.
- Custom Go eBPF agent; use the platform’s optional Beyla approach if ever needed.
- Custom executable JavaScript rule bodies and `isolated-vm`; retain declarative rules only.
- Shared-rule marketplace/distribution.
- Fake or weak scoring inputs: empty class inventories, guessed changed-line counts, zero duplication, and “any parsed file means API.”
- Duplicate GitHub webhook handlers.
- Public Bull Board.
- Product-owned AWS/GCP Terraform stacks; use the shared baukit chart/GitOps deployment.
- Committed Playwright reports/test-result state.
- Placeholder UI routes and placeholder data.

### Defer until the core proves reliable

- GitLab, Bitbucket and Azure DevOps adapters.
- Jira, Linear and GitHub Issues synchronization.
- Confluence/Docusaurus/Astro document ingestion.
- Qdrant, embeddings and semantic search.
- Runtime telemetry ingestion, service topology and static/runtime drift.
- LLM enrichment, code review, ADR consistency checking and auto-fix.
- ADR authoring/reviews/impact/supersession.
- Spec-source ingestion and spec-code traceability.
- Compliance frameworks, snapshots, overrides and evidence reports.
- PDF/report generation, schedules and object storage.
- Slack/Teams/email/PagerDuty notifications, digests and escalation.
- Global anonymized benchmarks and cohort percentiles.
- Technology radar/lifecycle/fragmentation beyond a simple detected dependency inventory.
- Audit S3 export and outgoing customer webhooks.
- Public API keys and machine-to-machine API access beyond the local CLI/GitHub Action.
- Air-gapped and licensed self-hosted distribution.
- Mobile application.

## 3. Target scaffold decision

Use:

```text
baukit new architecture-health-platform --backend --web --auth oidc
```

Do not pass `--mobile`.

A worker is mandatory, but the current `baukit new` CLI has no `--worker` switch and its backend template only renders `worker.enabled = false` in deployment values. Add the product-local worker crate/binary after proving the untouched generated scaffold. Log the missing generator capability as a platform candidate.

Recommended rewrite crates:

```text
backend/crates/
  architecture-health-platform-domain
  architecture-health-platform-ports
  architecture-health-platform-services
  architecture-health-platform-api
  architecture-health-platform-postgres
  architecture-health-platform-analysis
  architecture-health-platform-integrations
  architecture-health-platform-worker
  architecture-health-platform-bin
```

Frontend target:

- Vite + React + TanStack Router/Query + Tailwind 4.
- Generated OpenAPI client through `@baukit/api-runtime`.
- `@baukit/ui-tokens`.
- Structural analytics only; never send repository names, paths, source, finding text, diffs or credentials to product analytics.
- No Dexie/offline cache initially.
- Poll analysis status; defer SSE/WebSockets until polling is demonstrably insufficient.

## 4. Rewrite wave plan

### Wave G1 — scaffold + walking skeleton (sequential, 1 agent)

- [ ] Create a fresh orphan `baukit-rewrite` branch from `baukit new architecture-health-platform --backend --web --auth oidc`, pinned to `baukit-v0.2.0`; old code remains available only through existing history/reference branch, with no copied product files
- [ ] Generated backend/web CI, Docker-gated tests, OpenAPI drift, frontend lint/typecheck/test/build and `baukit doctor` green **before product code**
- [ ] Compose up PostgreSQL + Keycloak; browser PKCE login → generated protected `/me` → internal user bootstrap demonstrated
- [ ] Record every scaffold/manual step in the baukit Log, especially the missing worker generation capability

### Wave G2 — core domain + persistence (1 agent)

- [ ] Replace the generated example domain with `User`/`UserIdentity`, `Organization`/`OrganizationMember`, `Project`/`ProjectRepository`, `Repository`/`RepositoryConnection`, `AnalysisRun`, persistent `Finding`, `FindingOccurrence`, `FindingStatusHistory`, `Dependency`, `AnalysisMetric`, `HealthScore`, `RulesetConfig`, `QualityGateConfig`, `QualityGateEvaluation`, and `JobOutbox`
- [ ] Product crates follow generated dependency direction; domain is provider-free, ports define repositories/clock/ID generation/job dispatch, services own authorization and use cases, SQLx adapters own PostgreSQL
- [ ] Ordered migrations, constraints and tenant-scope indexes; no JSON dumping where normalized lifecycle/query fields are known
- [ ] Unit/property tests for membership scope, finding fingerprints, status transitions, automatic resolve/reopen, sortable IDs and health-history ordering; PostgreSQL integration tests via baukit-test

### Wave G3 — repository intake + worker foundation (1 agent after G2; ∥ G4)

- [ ] Add `architecture-health-platform-integrations`, `architecture-health-platform-worker` and worker bin; product-local durable Postgres job/outbox implementation with `repository.sync`, `analysis.run`, `analysis.schedule` and `webhook.process`
- [ ] Git provider port plus GitHub adapter: installation/token credentials, repository catalog, verified raw-body webhook signatures, platform-ID→internal-repository resolution, idempotent delivery IDs and scoped temporary clone credentials
- [ ] Manual/default-branch/scheduled scan triggers; bounded retry/backoff, per-repository concurrency exclusion, cancellation/timeout, temporary-directory cleanup and crash recovery
- [ ] Worker uses baukit runtime/telemetry/ops and exact §2.4 worker metric families, including queue age; integration tests use a local bare Git fixture and mocked GitHub HTTP/webhooks

### Wave G4 — scanner, parsers and dependency graph (1 agent after G2; ∥ G3)

- [ ] New Rust `architecture-health-platform-analysis` crate: ignore-aware scanner, file-size/line limits, deterministic path normalization, explicit supported/unsupported language capability matrix
- [ ] Tree-sitter parsers and import/entity/function extractors for TypeScript/JavaScript and Rust first, then Python/Java/Go behind isolated language modules with golden fixture repositories
- [ ] Directed dependency graph, strongly connected components/cycle canonicalization, module metrics and source locations; no analyzed repository code is executed
- [ ] Property tests for graph invariants/cycle stability plus golden tests for monorepos, aliases, relative imports, generated files, parse failures and oversized files

### Wave G5 — rules, scoring and analysis result contract (1 agent after G4)

- [ ] Versioned declarative rule catalog with the initial high-confidence structural/code-quality/security/reliability rules; rule IDs and finding fingerprints are stable public contracts
- [ ] Per-repository ruleset resolution supports enable/disable, severity and typed threshold overrides; custom executable code and shared marketplace are absent
- [ ] Versioned health scoring uses only present, measured components and exposes evidence/confidence; missing components redistribute weight explicitly and never become guessed values
- [ ] Analysis result contains scan summary, dependencies, findings, metrics, component/overall scores and topology; deterministic JSON snapshots and regression fixtures cover score/rule changes

### Wave G6 — analysis orchestration + REST/OpenAPI contract (1 agent after G3/G5)

- [ ] Worker transaction flow: claim job → clone → analyze → persist run/findings/dependencies/metrics/score → reconcile persistent findings → commit outbox events → clean workspace; idempotent retry cannot duplicate a completed run
- [ ] Authenticated API endpoints: `/me`; organizations/projects CRUD; repositories CRUD; `POST/GET /api/v1/repositories/{id}/analyses`; `GET /api/v1/analyses/{id}`; repository health/history/findings/topology/ruleset; finding status update; GitHub webhook
- [ ] Cursor pagination, organization-scoped authz on every query/mutation, standard error envelope, request IDs and documented conflict/rate/validation responses
- [ ] Deterministic committed OpenAPI and generated TypeScript client; API/worker/Postgres integration test proves trigger → job → completed analysis → query results

### Wave G7 — web product (1 agent after G6; ∥ G8)

- [ ] Vite/TanStack authenticated shell with organization/project selection and repository connection/onboarding
- [ ] Repository dashboard: analysis state, versioned overall/component score, trend, finding counts, top findings and manual re-run
- [ ] Findings table/detail/status workflow, health history, dependency/topology graph and per-repository ruleset editor; accessible loading/error/empty states replace all placeholder data
- [ ] `@baukit/ui-tokens`, generated API client and privacy-safe structural analytics; Vitest/Testing Library plus Playwright login → connect fixture repo → analyze → inspect finding/topology → change finding status

### Wave G8 — local CLI + CI integration (1 agent after G5; ∥ G7)

- [ ] Rust CLI commands `scan`, `analyze` and `gates` run the same analysis crate locally; JSON and human output, exclude/ruleset config, score threshold and stable exit codes
- [ ] GitHub Action wrapper installs/runs the CLI, restores safe caches, emits summary/annotations and sets pass/fail status without uploading source
- [ ] Quality-gate templates for strict, relaxed, monolith, modular-monolith and microservices are re-authored against the new rule IDs
- [ ] CLI golden output, exit-code tests and GitHub Action fixture workflow cover pass/fail, malformed config and below-threshold behavior

### Wave G9 — remediation + PR governance (1 agent after G6/G8)

- [ ] Persistent finding lifecycle reconciles occurrences by fingerprint, preserves acknowledgement/assignment/status history and automatically resolves/reopens only under tested rules
- [ ] GitHub pull-request job analyzes the real head/base, derives changed files and line counts from Git, computes finding delta/risk from actual graph data and posts bounded review comments
- [ ] Quality-gate evaluation is persisted per analysis with immutable config snapshot; PR status and CLI evaluate the same result, with false-positive annotation recorded but never mutating historical output
- [ ] End-to-end fixture covers webhook replay, PR synchronize, new/resolved/worsened findings, comment update/deduplication and blocking/non-blocking gates

### Wave G10 — telemetry + conformance gate (sequential, 1 agent after G7–G9)

- [ ] Baukit-test ops, auth, metrics and OpenAPI conformance suites wired for API and worker; gating readiness checks PostgreSQL and the durable job store
- [ ] Standard HTTP and worker metric families recorded exactly once; product metrics use `architecture_health_platform_` prefix and bounded repository-language/rule-category labels only
- [ ] Logs/traces scrub source, diffs, repository credentials, webhook bodies, access tokens and LLM/provider content; workspace paths and repository names are not metric labels
- [ ] Full CI-equivalent gate green: fmt, clippy, Rust unit/property/integration tests including Docker, coverage target, deny/MSRV, OpenAPI/client drift, frontend lint/typecheck/coverage/build, Playwright and CLI/Action fixtures

### Wave G11 — deploy + rewrite exit gate (sequential, 1 agent)

- [ ] Separate API, worker and migration images; worker image includes Git, parser/runtime assets and a bounded writable scratch volume, while the API image does not
- [ ] Shared baukit chart values enable API + worker + release migration, private ops listeners, PostgreSQL/Keycloak, worker egress to GitHub and default-deny network policy; no Redis/Qdrant/MinIO in the first deployment
- [ ] Deploy to disposable k3d/K3s; smoke OIDC login → connect seeded Git provider/fixture repository → analysis completes → dashboard renders → `/metrics` scraped → PR/CLI gate agrees
- [ ] Shared Grafana dashboard and burn-rate/worker alerts render unmodified; graceful shutdown proves API drain and in-flight job recovery
- [ ] Fresh restore/deploy rehearsal and documented rollback complete; old implementation remains reference history only and the baukit rewrite becomes the release branch

## 5. Baukit platform-gap candidates

Log these as candidates; do not solve them during G0.

| Candidate gap | Evidence/impact | Suggested disposition |
|---|---|---|
| Worker generation | CLI has no `--worker`; template chart only has `worker.enabled = false` and no worker crate/bin | Strong baukit candidate after this product and leitbild prove the same need |
| Durable job/outbox implementation | Baukit standardizes runtime/metrics but not storage, claiming, scheduling, idempotency or retry mechanics | Product-local initially; extraction review after two consumers |
| Queue readiness/age helpers | Product needs oldest-job age, stuck-job detection and readiness beyond generic process supervision | Candidate addition to worker conformance after a concrete implementation exists |
| Git repository workspace lifecycle | No shared clone credentials, temp workspace, cancellation, disk quota, cleanup or untrusted-repository conventions | Product-local security boundary; document operational requirements |
| Git provider/webhook test kit | No shared Git provider port, raw-body signature fixture or delivery-idempotency fixture | Product-local now; possible test-helper extraction if another product consumes it |
| Object-storage port | Analysis recommends one, but current baukit crates/templates do not provide it | Log for deferred reports/audit exports; not needed for core rewrite |
| Machine-to-machine authentication | Generated auth covers OIDC users, not scoped API keys/service credentials for CI clients | Defer; reassess before hosted/public REST API |
| Multi-tenant RBAC | Baukit maps OIDC subject to user but does not provide organization/project policy or scope enforcement | Correctly product-owned; no generic RBAC framework yet |
| Analysis worker deployment needs | Shared chart does not express bounded scratch workspace, Git/parser assets, job-specific egress or analysis concurrency/autoscaling | Add product values first; promote only reusable chart seams |
| Static-analysis fixtures/tooling | No shared tree-sitter grammar packaging, fixture repository harness or deterministic analysis snapshot convention | Product-owned domain capability |
| CLI/GitHub Action scaffold | Generator creates backend/web/mobile only | Product-local; log if a second product needs distributed CLI/action packaging |
| Streaming job progress | Template supplies request/response APIs, not SSE/WebSocket job progress | Polling is sufficient initially; do not expand baukit yet |
| LLM/vector/PDF/notification ports | Current platform intentionally leaves AI and product policy local; S3/email capabilities are planned but not implemented | Deferred product capabilities, not blockers |
| Secrets for provider installations | Baukit secret wrappers cover configuration, not per-organization encrypted provider credentials/rotation | Product-local adapter concern; avoid creating a generic vault abstraction prematurely |
| Large analysis-result persistence | No bulk-copy/object-spill conventions for very large dependency graphs/findings | Measure first; SQLx batching is sufficient until real limits appear |
