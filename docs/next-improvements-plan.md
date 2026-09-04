# Baukit next improvements implementation plan

**Status:** In execution since 2026-09-04. Claude orchestrates; codex agents (gpt-5.6-sol, high) implement and verify. Progress lives in the execution tracker below. Commits go straight to `main`; no pull requests, and the release is prepared locally for manual publishing.

**Prepared:** 2026-09-04

**Baukit baseline:** `88f513be8ea771920471c88a0007a479470954a1`, release `0.2.1`

## Execution tracker

Legend: `[ ]` not started, `[~]` in progress, `[x]` done in Baukit, `[-]` decided not to implement (see log). Product adoption is tracked as a follow-up per item and happens in the product repositories, not here.

Batches group items that can run in parallel with disjoint file ownership. Each batch ends with a re-bless of CLI snapshots when templates changed and a CI-equivalent verification run.

### Wave 1

- [ ] 1. JSON rejection classes in `baukit-http` (batch 1)
- [ ] 2. Typed `ApiTokenStoreError` in `baukit-auth` and `baukit-test` (batch 1)
- [ ] 3. Principal-establishing middleware, authentication before rate limiting (batch 2)
- [ ] 4. Named authenticated route-group limits (batch 3, after 3)
- [ ] 5. Bounded terminal-job cleanup in `baukit-jobs` (batch 1)
- [ ] 6. Fixed recurring-slot helpers in `baukit-jobs` (batch 1)
- [ ] 7. Production resource-budget measurements, Rust and TypeScript, shared fixtures (batch 1)

### Wave 2

- [ ] 8. Overlapping sync-response conformance (batch 2)
- [ ] 9. `@baukit/sync-client/browser` scheduler environment (batch 2)
- [ ] 10. Serialized preference updates (batch 2)
- [ ] 11. Cross-runtime hybrid logical clock (batch 3)
- [ ] 12. Supported PWA worker artifact (batch 2)

### Wave 3

- [ ] 13. Browser QA configuration backport (batch 3)
- [ ] 14. `.env` reconciliation script (batch 4)
- [ ] 15. Local Markdown link check in the strict profile (batch 4)
- [ ] 16. Parameterized Keycloak realm policy validation (batch 4)
- [ ] 17. Idempotent development-realm reconciler (batch 4, after 16)
- [ ] 18. Script-only accessible Keycloak theme (batch 5)
- [ ] 19. Localization and identity helpers: catalog segments, request locale extractor, identity hints, UUIDv7 study (batch 3)

### Wave 4

- [ ] 20. `@baukit/auth-node` device-flow package (batch 4)
- [ ] 21. Opt-in MCP capability and generator (batch 5)

### Wave 5

- [ ] 22. Tombstone horizon and full-resync contract plus conformance callbacks (batch 5)
- [ ] 23. Durable-job ownership design note (batch 6)
- [ ] 24. Provider credential-probe contract in `baukit-integrations` (batch 5)
- [ ] 25. Import-envelope conformance (batch 6)
- [ ] 26. Inbox and webhook reliability recipes and helpers (batch 6)
- [ ] 27. Raw OpenAPI mirrors decision note (batch 6)
- [ ] 28. Live-row cap PostgreSQL recipe and concurrency helpers (batch 6)

### Wave 6 studies

Each study ends in one written decision under `docs/studies/`.

- [ ] 29. Revisioned write queue and durable form drafts (batch 7)
- [ ] 30. Offline asset management (batch 7)
- [ ] 31. Expo UI and headless accessibility behavior (batch 7)
- [ ] 32. Notifications and timeline playback (batch 7)
- [ ] 33. Calendar export (batch 7)
- [ ] 34. Release, GitOps, and migration compatibility (batch 7)
- [ ] 35. Browser identity composition (batch 7)
- [ ] 36. Other deferred contracts review (batch 7)

### Release

- [ ] Full CI-equivalent verification on `main` after the last batch
- [ ] Release train prepared locally (minor bump, `ApiTokenStore` is a breaking change), compatibility matrix updated, coherence check green, tag created locally
- [ ] Manual publish steps handed over (npm, crates.io, tag push)

### Product adoption follow-ups

Filled in as items complete. Each line names the product file to delete once the product pins the released train.

### Log

- 2026-09-04: Tracker added. Batch plan fixed: batch 1 = items 1, 2, 5+6, 7; batch 2 = items 3, 8+9, 10, 12; batch 3 = items 4, 11, 13, 19; batch 4 = items 14+15, 16+17, 20; batch 5 = items 18, 21, 22, 24; batch 6 = items 23+27, 25, 26, 28; batch 7 = studies 29 to 36; then release preparation.

## Purpose

This plan combines the playback audits from Tiefgang, Leitbild, Redemut, and Eigenruhe into one Baukit backlog. It turns repeated findings into ordered, reviewable changes and records what must remain in the products.

The four audits reach the same broad conclusion. Baukit already owns most of the shared application mechanics. The next work should close specific gaps in those mechanics, add a few opt-in capabilities, and test repeated behavior without importing product models. Large application subsystems should stay where they are.

The source audits are:

- Tiefgang: `/home/patrick/projects/tiefgang/docs/baukit-playback-audit.md`, revision `861cf0a994d5e63ec245e645023c80575759c191`
- Leitbild: `/home/patrick/projects/leitbild/docs/baukit-replay-assessment.md`, revision `25eda07`
- Redemut: `/home/patrick/projects/redemut/docs/baukit-playback-audit.md`, revision `b4e8a98`
- Eigenruhe: `/home/patrick/projects/eigenruhe/docs/BAUKIT_FEEDBACK_AUDIT.md`, revision `36b468d015f4aebd83a11bd662c7ff82124711fb`

All four audits evaluated the same Baukit `0.2.1` baseline. This plan therefore does not need to account for intervening Baukit changes.

## Result

Work should proceed in six waves:

| Wave | Goal | Main result |
| --- | --- | --- |
| 1 | Repair current Rust contracts | HTTP rejection classes, typed token storage failures, supported authenticated rate limiting, job cleanup, recurring slots, and runtime budget measurements |
| 2 | Fix client concurrency and runtime gaps | Sync race conformance, a browser scheduler environment, serialized preference updates, hybrid logical clocks, and a supported service-worker build |
| 3 | Improve generated projects and identity setup | Browser QA flexibility, environment reconciliation, documentation links, Keycloak policy and development reconciliation, and the existing accessible-theme decision |
| 4 | Add agent-facing capabilities | A secure Node authentication package followed by an opt-in MCP package generator |
| 5 | Specify data lifecycle and integration behavior | Tombstone horizons, job ownership, provider credential probes, import conformance, inbox and webhook recipes, and OpenAPI schema mirrors |
| 6 | Run bounded cross-product studies | UI behavior, autosave, drafts, offline assets, notifications, timeline playback, calendars, release tooling, content bundles, and other deferred candidates |

Waves describe dependency order. They are not release numbers. Each numbered work item below should remain a separate pull request unless the item explicitly says otherwise.

## How the audits were combined

Repeated findings carry more weight, but repetition is not enough on its own. A proposal moves early only when Baukit already owns the affected contract or the proposed addition has a small, product-neutral interface.

| Combined topic | Product evidence | Decision |
| --- | --- | --- |
| Authentication before identity rate limiting | Leitbild, Eigenruhe | Implement first as one supported composition |
| Named authenticated route groups | Eigenruhe, supported by Leitbild's composition gap | Implement with the authentication composition |
| Node device authorization | Tiefgang, Leitbild, Eigenruhe | Build an optional Node package after protocol and cache hardening |
| MCP package generation | Tiefgang, Leitbild, Eigenruhe, Redemut | Add an opt-in capability with explicit tools; do not expose OpenAPI routes automatically |
| Expo UI and accessibility behavior | Tiefgang, Eigenruhe, Redemut, plus the earlier Fitness Tracker evidence | Start the deferred comparison; extraction is not pre-approved |
| Keycloak behavior and tooling | Leitbild, Redemut, Tiefgang | Implement policy and development tooling; finish the existing script-only accessibility spike separately |
| Sync correctness | Tiefgang, Eigenruhe, Redemut | Add race cases now, add a browser adapter now, and specify purge horizons before adding server helpers |
| Runtime resource-budget measurement | Eigenruhe, with compatible product implementations elsewhere | Move measurement into production libraries while products keep limits and reason codes |
| iCalendar behavior | Redemut, with caution from Eigenruhe | Run a dependency and parity study before accepting maintenance of an encoder |
| Provider clients | All four products | Keep mappings local; add only the credential-probe contract with two proven adapters |
| Import and local artifact safety | Eigenruhe, Redemut, Tiefgang | Add conformance where the safety rule is neutral; do not own product schemas |
| Release and GitOps tooling | Leitbild | Start with a manifest, validation, and dry-run plan; do not move environment authority into Baukit |

Two disagreements need an explicit resolution.

First, MCP authentication must not dictate the MCP generator schedule. Redemut favors personal access tokens for stdio, while Tiefgang, Leitbild, and Eigenruhe have device-flow implementations. The generator should initially accept an injected bearer-token provider and include a documented personal-token mode. `@baukit/auth-node` can become an optional adapter after its security gates pass.

Second, Baukit should not commit to maintaining its own iCalendar encoder based on Redemut alone. The calendar study must compare maintained dependencies with the two Redemut implementations. A Baukit implementation is justified only if no suitable dependency meets the Rust, TypeScript, timezone, and deterministic-test requirements.

## Rules that apply to every wave

### Preserve package boundaries

- Baukit owns mechanics, conformance, and generated composition.
- Products own entities, schemas, SQL, routes, scopes, quotas, conflict choices, copy, visual design, and provider mappings.
- Optional capabilities must not add their dependencies to products that do not select them.
- A package must work without importing a product name or adding product switches.
- When a behavior is mostly policy, prefer a contract, recipe, or test adapter over a runtime package.

### Require an adoption path

Every implementation pull request must name the product code it is intended to replace. After a Baukit release, the source product should adopt the public release and delete its duplicate. Workspace path overrides do not count as adoption proof.

For a new abstraction based on one product, require either a second consumer or a generated fixture that proves the interface. A fixture can prove package independence. It cannot prove that product policy was separated correctly when the audits explicitly call for a second product.

### Keep compatibility visible

Each public API change needs:

- a changelog entry;
- a README or platform-contract update;
- a migration note for current consumers;
- a compatibility test for the previous behavior when that behavior remains supported; and
- a version-coherence check before release.

`ApiTokenStore` changes every product adapter, so its typed error is a planned breaking API change. The JSON extractor and preference controller should use additive configuration first. Do not silently change current error codes or update visibility.

### Test distributed artifacts

Tests must cover the thing consumers install, not only source files. This matters for package exports, the PWA worker build, generated MCP packages, Keycloak theme assets, CLI output, and OpenAPI mirrors.

### Use one evidence record per item

Before implementation begins, open an issue or design note with:

- source files in each product;
- the observed failure or repeated glue;
- the Baukit owner;
- the proposed public types and errors;
- product-owned inputs;
- concurrency, failure, privacy, and cleanup cases;
- supported runtimes; and
- the product adoption pull request that will remove the duplicate.

## Wave 1: repair current Rust contracts

Wave 1 fixes behavior that already belongs to a Baukit crate. These changes do not create new product concepts.

### 1. Preserve JSON rejection classes in `baukit-http`

**Evidence:** Eigenruhe maintains a local extractor because `ApiJson<T>` maps every Axum JSON rejection to one `400` response. This loses `413 Payload Too Large`, which the resource-budget contract requires clients to distinguish.

**Target files:**

- `rust/crates/baukit-http/src/extract.rs`
- `rust/crates/baukit-http/src/error.rs`
- `rust/crates/baukit-http/src/options.rs`
- `rust/crates/baukit-http/src/tests.rs`
- `rust/crates/baukit-http/README.md`
- `rust/crates/baukit-http/CHANGELOG.md`

**Design:**

- Classify Axum rejections as body too large, missing or invalid content type, syntax error, and data-shape error.
- Preserve Axum's status where it carries protocol meaning, especially `413` and `415`.
- Map each class to a safe Baukit error envelope without returning body text or serde internals.
- Add an options type for class-specific stable codes. Preserve the current single-code setting as a compatibility path for one release cycle.
- Keep route-specific body limits and user-facing text in products.

**Tests:**

- malformed JSON;
- missing content type;
- unsupported content type;
- a field type mismatch;
- a body above the configured limit;
- request ID propagation;
- no submitted body or parser detail in logs or responses; and
- compatibility mode returning the configured legacy code.

**Done when:** Eigenruhe can delete its custom JSON extractor without changing its public `payload_too_large` behavior.

### 2. Replace string storage failures in `baukit-auth`

**Evidence:** Leitbild encodes an active-token limit as `limit_exceeded:api_tokens_active:N` inside `String`, then parses it in its API crate. Current `ApiTokenStore` methods all return `String` failures.

**Target files:**

- `rust/crates/baukit-auth/src/api_token.rs`
- `rust/crates/baukit-auth/src/lib.rs`
- `rust/crates/baukit-auth/README.md`
- `rust/crates/baukit-auth/CHANGELOG.md`
- `rust/crates/baukit-test/src/api_token.rs`
- `rust/crates/baukit-test/README.md`
- authenticated backend template adapters and conformance tests

**Design:**

- Add `ApiTokenStoreError` with an unavailable or internal variant and a safe policy-rejection variant.
- Let a policy rejection carry a validated snake-case code plus bounded numeric details. It must not carry arbitrary SQL or provider text into an API response.
- Change every `ApiTokenStore` operation to return the typed error.
- Preserve authentication probing resistance. Malformed, unknown, hash-mismatched, and revoked tokens still produce the same public authentication result.
- Map unexpected adapter errors to `ApiTokenError::Storage` without exposing their text through public responses.

**Tests:**

- structured active-token limit failure;
- unexpected SQL-like detail remains private;
- malformed, unknown, wrong-hash, and revoked credentials remain indistinguishable;
- expired tokens retain the existing result;
- the `baukit-test` memory store can script both typed error classes; and
- every generated and in-repository adapter compiles after the trait change.

**Migration:** This is a public trait break. Update all known app adapters in their adoption pull requests and call it out in the release notes.

**Done when:** Leitbild no longer parses storage error strings.

### 3. Support authentication before rate limiting

**Evidence:** Leitbild and Eigenruhe both authenticate in custom outer middleware so `baukit-ratelimit` can see a verified `Principal`. The current extractor caches a principal only after a route extractor runs, which is too late for an outer limiter.

**Target files:**

- `rust/crates/baukit-auth/src/axum_integration.rs`
- `rust/crates/baukit-auth/src/lib.rs`
- `rust/crates/baukit-ratelimit/src/axum_layer.rs`
- `rust/crates/baukit-ratelimit/src/options.rs`
- both crate READMEs and changelogs
- `templates/backend/backend/crates/__app__-bin/src/bin/api.rs`
- authenticated generated integration tests

**Design:**

- Add supported middleware that inspects a bearer credential, verifies it once, and stores `Principal` in request extensions.
- Distinguish no credential from an invalid credential. No credential may continue to an IP safety limit when the route permits anonymous traffic. Invalid credentials return authentication failure before consuming an anonymous bucket.
- Make the existing `Principal` extractor reuse the cached value, as it does today.
- Accept any `IdentityVerifier`, including an OIDC-only verifier or a product composition that supports both OIDC and Baukit personal tokens.
- Do not connect Redis when both rate-limit scopes are disabled.
- Document Axum layer order with one generated composition rather than prose alone.

**Tests:**

- two valid subjects consume separate identity buckets;
- an anonymous request uses only the IP bucket;
- an invalid and an expired token return the existing authentication envelope;
- route extraction does not verify a credential twice;
- OIDC and personal-token verifiers work through the same middleware;
- disabled limiting does not require a Redis URL or connection;
- fail-open and fail-closed store behavior remains intact; and
- proxy-derived IP handling remains unchanged.

**Done when:** Leitbild and Eigenruhe can remove their principal-caching middleware and generated authenticated backends demonstrate the supported order.

### 4. Add named authenticated route-group limits

**Evidence:** Eigenruhe repeats store delegation, subject key construction, write predicates, response headers, retry details, and error mapping for several authenticated route groups.

**Dependency:** Complete item 3 first so the group layer can rely on a verified principal.

**Target files:**

- `rust/crates/baukit-ratelimit/src/axum_layer.rs`
- `rust/crates/baukit-ratelimit/src/options.rs`
- `rust/crates/baukit-ratelimit/src/store.rs`
- `rust/crates/baukit-ratelimit/src/lib.rs`
- crate tests, README, and changelog
- one authenticated backend template example

**Design:**

- Add a layer configured with a validated group name, quota, subject-key extractor, and request predicate.
- Namespace group counters separately from global identity and IP counters.
- Return `Retry-After`, `RateLimit-Limit`, `RateLimit-Remaining`, and `RateLimit-Reset` on rejection.
- Add safe `retry_after` detail to the standard error body so generated TypeScript clients do not need to infer it from prose.
- Allow one concrete store to implement both request-count and amount-budget traits without a product-owned delegation enum.
- Keep group names, limits, method selection, and product metric names in the application.

**Tests:**

- two identities and two groups have independent counters;
- the predicate bypasses requests that should not count;
- unsafe or unbounded group names are rejected during setup;
- memory and Redis stores behave alike;
- headers and body retry details agree;
- fail-open and fail-closed behavior; and
- no identity value becomes a metric label.

**Done when:** Eigenruhe can replace its route-group wrapper with configuration and a small product key or predicate function.

### 5. Add bounded terminal-job cleanup

**Evidence:** Tiefgang deletes rows from Baukit's `job_outbox` using Baukit status strings. The crate owns the schema and state machine but has no retention operation.

**Target files:**

- `rust/crates/baukit-jobs/src/model.rs`
- `rust/crates/baukit-jobs/src/store.rs`
- `rust/crates/baukit-jobs/src/lib.rs`
- `rust/crates/baukit-jobs/tests/postgres.rs`
- crate README and changelog
- generated worker documentation

**Design:**

- Add explicit cutoffs for succeeded, cancelled, and failed rows.
- Require a nonzero batch limit and delete no more than one batch per call.
- Return counts by terminal status.
- Never delete pending or running rows, including expired running leases.
- Prefer a `PostgresJobStore` operation rather than widening `JobStore` unless worker-generic code needs the method.
- Leave retention periods, invocation schedule, shutdown, and product-table cleanup in applications.

**Tests:**

- independent cutoffs for all terminal states;
- pending and running rows survive;
- an expired lease still survives;
- repeated bounded calls converge;
- a concurrent claim or completion does not delete an active row;
- zero and excessive batch limits are rejected or bounded according to the public contract; and
- returned counts match committed deletes.

**Done when:** Tiefgang can delete its direct `job_outbox` cleanup SQL.

### 6. Add fixed recurring-slot helpers to `baukit-jobs`

**Evidence:** Eigenruhe has two jobs that independently calculate the next UTC slot and construct an idempotency key before self-enqueueing.

**Target files:**

- a small new module under `rust/crates/baukit-jobs/src/`
- `rust/crates/baukit-jobs/src/lib.rs`
- crate tests, README, and changelog
- the generated worker recipe

**Design:**

- Provide pure fixed-interval UTC slot calculation and a canonical slot identifier.
- Define delayed execution as scheduling the next wall-clock slot, not one interval after completion.
- Do not add cron parsing, a scheduler service, product job names, or catch-up policy.
- Show how to enqueue the next slot with the existing idempotency and transaction rules.

**Tests:**

- exact-boundary time;
- delayed execution;
- multiple missed slots;
- backward and forward clock movement;
- duplicate delivery and process restart; and
- enqueue failure leaves the current handler outcome explicit.

**Done when:** Eigenruhe can share the slot calculation while keeping notification and retention cadence local.

### 7. Publish production resource-budget measurements

**Evidence:** Eigenruhe implements matching Rust and TypeScript production functions for Unicode scalar counts, compact JSON UTF-8 bytes, byte lengths, and collection lengths. Baukit currently keeps the Rust measurements in `baukit-test`, even though applications need them in write paths.

**Target placement:** Decide whether the Rust functions belong in `baukit-core` or a small new crate. Put TypeScript functions in a small runtime-neutral package or an existing package only if that package's stated responsibility fits. `baukit-test` should reuse the production functions rather than own a second implementation.

**Design:**

- Publish measurements, not a product policy language.
- Return measured and allowed values from check helpers so products can map their own reason codes.
- Define Unicode scalar counting, trimming, compact JSON encoding, unsupported JavaScript values, non-finite numbers, and object-key behavior.
- Add a shared fixture corpus read by Rust and TypeScript tests.
- Keep `limits.json`, limit values, operation mapping, error codes, and UI copy product-owned.

**Tests:**

- ASCII, composed and decomposed Unicode, emoji sequences, and unpaired JavaScript surrogates;
- compact objects, arrays, escapes, and multibyte strings;
- non-finite or unsupported JavaScript input;
- bytes and collection boundaries;
- fixture parity between Rust and TypeScript; and
- `baukit-test` conformance calling the production functions.

**Done when:** Eigenruhe can replace both local measurement modules and Baukit has only one implementation per language.

## Wave 2: client concurrency and runtime gaps

### 8. Add overlapping sync-response conformance

**Evidence:** Tiefgang has a regression test for a delayed rejected response arriving after a newer local write. Its Dexie and SQLite adapters guard both accepted and rejected late responses. Eigenruhe also identifies outcome settlement and purge reset as missing conformance cases.

**Target files:**

- `typescript/packages/sync-client/src/conformance.ts`
- `typescript/packages/sync-client/src/conformance.test.ts`
- `typescript/packages/sync-client/type-tests/conformance.ts`
- `typescript/packages/sync-client/README.md`
- `docs/platform/offline-readiness-contract.md`
- package changelog

**Design:**

- Extend the adapter with an atomic submitted-batch outcome operation. The operation receives the exact pending rows covered by that submission.
- Keep old adapter methods available during migration if the new tests cannot be expressed through them.
- State revision-stamping behavior explicitly instead of assuming one conflict algorithm.
- Keep entity payloads, server conflict choices, dependency ranks, rejection copy, and repair actions in products.

**Required cases:**

1. Submit A, create and submit B, then apply A's accepted result. Only A's pending ID disappears and B remains the visible local value.
2. Submit A, create and submit B, then apply A's rejected result with a server row. B remains pending and visible.
3. Pull an older or equal remote revision while a local write is pending. The local row and pending count remain unchanged while the page and cursor commit atomically.
4. Fail local application after staging a row. The row and cursor both roll back.
5. Return incomplete push coverage. No submitted outcome is partially acknowledged.

**Adoption proof:** Run the suite against Tiefgang's Dexie and Expo SQLite adapters. Add Eigenruhe if its callback mapping is small enough without changing its sync engine.

**Done when:** the shared suite reproduces the Tiefgang race and both real storage adapters pass it.

### 9. Add `@baukit/sync-client/browser`

**Evidence:** Redemut supplies the missing browser adapter for visibility, online events, and timers. Baukit already provides the equivalent Expo entry.

**Target files:**

- `typescript/packages/sync-client/src/browser.ts`
- `typescript/packages/sync-client/src/browser.test.ts`
- package exports, type-export tests, README, and changelog

**Design:**

- Implement `SyncSchedulerEnvironment` with injected or global `document`, `window`, and timer functions.
- Treat hidden-to-visible and browser online events as scheduler wake signals.
- Make subscription cleanup idempotent.
- Do not import React, Dexie, a product sync engine, or browser globals at module evaluation time.
- Decide whether retry wake-up belongs in the scheduler API. Do not copy Redemut's decorator until that decision is explicit.

**Tests:** hidden and visible startup, visibility transition, online event, timer delegation, cleanup, repeated cleanup, absent globals, and package import in a Node test process.

**Done when:** Redemut imports the Baukit browser entry and deletes its scheduler environment.

### 10. Add explicit serialized preference updates

**Evidence:** Redemut wraps `@baukit/preferences-core` because concurrent optimistic updates can read the same prior state and settle out of order.

**Target files:**

- `typescript/packages/preferences-core/src/controller.ts`
- `typescript/packages/preferences-core/src/controller.test.ts`
- `typescript/packages/preferences-core/src/index.ts`
- package README and changelog

**Design:**

- Add an explicit update policy such as `updateMode: "serialized"`, or a separate serialized controller constructor.
- Do not change the default optimistic behavior in the first release.
- Define whether visible values are optimistic or committed for the serialized mode. Redemut's evidence supports committed visibility with `pendingCount`.
- Queue each patch against the latest committed result.
- Continue processing after a failed write without leaking the failed value into the next patch.
- Invalidate queued and in-flight publication after identity change or `stop()`.
- Preserve the existing side-effect and rollback contract.

**Tests:** rapid writes, out-of-order store completion, failed first write, side-effect rollback, identity switch, stop during a write, unknown keys, and pending-count transitions.

**Done when:** Redemut can delete `SerializedRedemutPreferenceController` and existing consumers retain current behavior unless they opt in.

### 11. Add a cross-runtime hybrid logical clock

**Evidence:** Redemut has matching Rust and TypeScript clocks backed by one fixture corpus. The clock is neutral, while Redemut's last-writer-wins merge policy is not.

**Target files:**

- a new HLC module in `rust/crates/baukit-sync`
- a new HLC module in `typescript/packages/sync-client`
- one shared fixture corpus in a repository test-data location
- both package READMEs and changelogs

**Design:**

- Publish encode, decode, compare, open, now, observe, and snapshot behavior.
- Keep time and random or persistence dependencies injected.
- Serialize concurrent TypeScript calls when state persistence is configured.
- Keep IndexedDB, SQLite, device IDs, merge rules, server compare-and-swap, and cursor protocols out of the modules.
- Document the JavaScript safe-integer bound and rollover behavior.

**Tests:** stalled time, backward time, logical rollover, restoration, remote observation, concurrent calls, corrupt stored state, maximum encoded value, and direct Rust to TypeScript fixture parity.

**Done when:** Redemut consumes both Baukit implementations and retains only its merge and persistence adapters.

### 12. Add a supported PWA worker artifact

**Evidence:** Eigenruhe rewrites compiled `@baukit/pwa-web` code with regular expressions before it can build a service worker.

**Target files:**

- `typescript/packages/pwa-web/package.json`
- package build configuration and a worker-specific entry
- artifact-level tests
- package README and changelog
- PWA template scripts and `baukit doctor` checks where applicable

**Design:**

- Publish a worker-safe ESM entry, IIFE artifact, or supported bundler function. Choose one after testing the current web and Expo-web build paths.
- Keep the worker entry free of Node, DOM-page, React, and React Native dependencies.
- Provide supported registration and cache-version migration examples.
- Document identity changes and logout cleanup for private caches.
- Keep cache names, routes, manifest metadata, icons, offline route selection, and partition keys in products.
- Treat adding PWA generation for Expo web as a separate CLI decision. The current manifest requires the web capability, and this package fix should not silently change that rule.

**Tests:** import the published artifact in a worker-like environment, route cache decisions, cache-version migration, identity switch, offline response, package tarball contents, and generated `build:sw:check`.

**Done when:** Eigenruhe's worker build uses a normal supported import and contains no source rewriting.

## Wave 3: generated quality and identity setup

Wave 3 changes templates and the CLI. Every item must regenerate golden trees and pass the generated fixture gates.

### 13. Backport safe browser QA configuration

**Evidence:** Leitbild extended the generated browser harness after Baukit `0.2.1`. Its useful additions support authenticated routes and more varied real forms. Some changes weakened existing checks, so this is a selective backport.

**Target files:** `templates/web/web/e2e/`, the web package scripts, CLI snapshots, and generated fixture tests.

**Add:**

- authenticated flags for routes, overlays, submissions, and route states;
- per-case API stubs;
- regular-expression headings;
- configurable control roles and exact target names;
- route-specific screen selectors and optional scroll checks;
- multiple required fields plus the expected invalid field;
- button or link recovery actions;
- dialog-scoped initial-focus lookup;
- bounded keyboard search with useful diagnostics;
- overflow diagnostics naming the largest elements; and
- a Playwright server working directory derived from the config location.

**Retain:**

- the delayed-first-load route-state check;
- an explicit skip when a second account is unavailable;
- inert-background checks where the browser exposes the semantics;
- the capability-specific console allowlist; and
- the current browser and breakpoint coverage.

**Done when:** both authenticated and unauthenticated generated fixtures pass without product-specific selectors in the shared tests.

### 14. Reconcile new `.env` keys without overwriting local values

**Evidence:** Eigenruhe has a tested script that appends assignments missing from `.env` while preserving existing local choices.

**Target files:** common template setup scripts, relevant README instructions, CLI snapshots, and script tests.

**Contract:** preserve all existing bytes, append only missing keys in `.env.example` order, handle `export`, comments, blank and quoted values, report key names without values, define duplicate-key behavior, and remain idempotent.

Do not fold Eigenruhe's process supervisor or fixed port list into this item. A parameterized local supervisor can be considered after another generated product repeats it.

**Done when:** a generated project can gain a new example variable without replacing local secrets or edits.

### 15. Generate a local Markdown link check

**Evidence:** Redemut's dependency-free checker validates local inline and reference links in repository documentation.

**Target files:** common strict-quality scripts, strict workflow and quality gate, CLI doctor expectations, snapshots, and fixture tests.

**Contract:** check committed Markdown under configured local roots, resolve relative files, ignore external URLs, print the source and missing target, and behave the same locally and in CI. Anchor validation can be a later addition.

**Done when:** broken local documentation links fail the generated strict gate.

### 16. Add parameterized Keycloak realm policy validation

**Evidence:** Redemut found missing policy checks in the generated realm. The current product checker guesses environment class from the realm name, which is not acceptable upstream.

**Target files:** authenticated backend template scripts, generated realm tests, `baukit doctor`, strict quality wiring, and authentication documentation.

**Contract:** accept an explicit environment class and validate password bounds, username or email exclusion as configured, brute-force protection, TLS policy, public-client PKCE, disabled direct-access grants, and bounded redirect URIs.

Products retain registration choice, exact password rules, realm names, clients, and test accounts. The checker validates declared policy; it does not impose one production policy on every product.

**Done when:** generated development and production fixtures declare their class explicitly and the checker rejects deliberately weakened fixtures.

### 17. Generate an idempotent development-realm reconciler

**Evidence:** Redemut needs reconciliation when a Keycloak volume survives changes to ports, clients, users, or realm JSON.

**Dependency:** Define the ownership split in item 16 first.

**Target files:** authenticated backend template, Compose setup, script tests, generated README, snapshots, and fixture checks against a real pinned Keycloak container.

**Contract:** merge active development origins and redirects, create or update selected users and public clients, preserve product-owned fields, avoid routine password resets unless requested, redact secrets, and remove a temporary recovery administrator after success or failure.

**Tests:** fresh realm, stale volume, changed port, missing user, changed client, lost administrator, interrupted recovery, repeated run, and cleanup failure.

**Done when:** `make dev` can reconcile a generated development realm without deleting its volume.

### 18. Finish the script-only accessible Keycloak theme decision

**Evidence:** Leitbild and Tiefgang add further product examples to the existing Fitness Tracker and OpenDialog study. They confirm the useful behavior and the maintenance cost of copied FreeMarker templates.

**Authority:** Follow `docs/platform/keycloak-accessible-theme.md`. Do not redesign this work in the implementation pull request.

**Deliver together:**

- unbranded `keycloak.v2` child theme with `theme.properties` and one script;
- no copied `login.ftl`, `register.ftl`, or other full page template;
- fake-DOM unit tests;
- real-browser tests against each exact supported Keycloak patch;
- a neutral product-child overlay fixture;
- realm selection and read-only Compose mount in the generated authenticated fixture; and
- production packaging guidance for the Operator path.

If inherited markup cannot support the required login and registration behavior, stop. Publish the compatibility and test recipe instead of shipping template copies.

**Done when:** every acceptance test already listed in the theme decision passes against the generated fixture.

### 19. Add small localization and identity helpers

These are independent pull requests that can share one wave.

**Typed catalog segments:** Move Redemut's generic catalog segment type into `@baukit/localization-core`. Use a reference locale to enforce exact keys and string versus plural-message shape. Keep supported locales, catalogs, and i18next adapters local.

**Request locale extractor:** Add a configured Axum extractor in `baukit-http` only after implementing percent-decoded query parsing and quality-weighted `Accept-Language` selection. Inputs are a supported-locale set, fallback, and query-override rule. Bound header and query input, define tie order, and reject malformed values predictably.

**Display-only identity hints:** Add one dependency-free TypeScript helper shared by web and native. Its name and docs must say that decoded JWT claims are unverified display hints. It may choose a display name and initials. It must never supply authorization, storage partition, or analytics identity. Products supply fallback text.

**Client UUIDv7:** First test maintained dependencies in Expo, browsers, and service workers. If one works without unwanted polyfills, document and pin it. Only add a runtime-neutral function to `@baukit/data-contracts` if the dependency study fails. Any Baukit function needs injected time and random bytes plus published RFC vectors.

## Wave 4: Node authentication and MCP generation

### 20. Build `@baukit/auth-node`

**Evidence:** Tiefgang, Leitbild, and Eigenruhe independently implement OIDC device authorization, PKCE, polling, refresh, and a local token cache.

**Package name:** Prefer `@baukit/auth-node` because the existing auth package family is named by runtime. Export a CLI-oriented device-flow entry. Confirm the name during the package API review.

**Boundaries:**

- Protocol core handles discovery, device authorization, S256 PKCE, polling, refresh rotation, and stable errors.
- Cache code handles profiles, atomic replacement, restrictive permissions, corruption, and locking.
- Presentation stays behind callbacks for verification URI, user code, status, and browser launch.
- Applications inject issuer, client ID, scopes, audience, cache namespace and path, fetch, clock, sleep, abort signal, and environment-token source.

**Security gates:**

- exact configured issuer or explicit allowlist;
- HTTPS endpoints except an explicit loopback development policy;
- issuer and discovered endpoint relationship validation;
- bounded discovery and token response bodies;
- per-request and total login timeouts;
- abort support;
- single-flight refresh within a process and a cache lock across processes;
- old cache preservation if replacement fails;
- refresh-token preservation when a rotation response omits one;
- symlink and permission handling;
- no token or raw provider response in logs or errors; and
- display-only treatment of locally decoded claims.

**Tests:** pending, slow-down, denial, expiry, malformed numeric fields, zero interval, optional refresh token, rotation, issuer mismatch, endpoint policy, oversized responses, abort, timeout, concurrent refresh, interrupted write, corrupt cache, permission failure, symlink handling, logout, and platforms without POSIX modes.

**External proof:** Run conformance against the pinned Keycloak version and one other standards-compliant issuer before the first stable release.

**Done when:** at least two product MCP or CLI clients replace their local auth modules with the package.

### 21. Add an opt-in MCP capability

**Evidence:** Tiefgang, Leitbild, and Eigenruhe have the same package outline. Redemut confirms the need but exposes security problems in a different transport and cache design.

**Dependency:** The first generator does not depend on item 20. It accepts a bearer-token provider and documents personal access tokens. Add `@baukit/auth-node` as an optional auth choice after that package passes its gates.

**CLI and manifest design:**

- Add `--mcp` and `capabilities.mcp` only for projects with a backend and TypeScript workspace support.
- Let authentication be `personal-token`, `node-oidc`, or caller-supplied according to selected capabilities.
- Register generated OpenAPI declarations through `openapi.consumers`.
- Keep raw schema copies out of the initial capability unless item 27 is complete.
- Extend doctor, lockfile generation, CI, snapshots, and generated-fixture checks.

**Generated package:**

- isolated TypeScript package with build, lint, typecheck, and test commands;
- stdio bootstrap that reserves stdout for protocol messages;
- graceful shutdown and `--help`;
- typed OpenAPI client seam;
- explicit read and write tool registries;
- one harmless example tool and, if useful, one resource example;
- outcome-only stderr logging;
- in-memory transport tests;
- annotations required for every registered tool;
- a product-owned tool-to-route allowlist checked against OpenAPI; and
- documentation generated from the registry rather than arbitrary source-string scans.

**Do not generate:** one tool per OpenAPI operation, product tool names, destructive defaults, product scopes, consent language, domain recovery copy, or raw backend exceptions.

**Tests:** protocol-clean stdout, malformed input, safe error conversion, annotation completeness, read and write separation, OpenAPI path drift, docs drift, shutdown, auth failure, no credential logging, package build, and generated fixture execution.

**Done when:** a generated fixture passes the full package gate and at least one existing product replaces its package bootstrap while retaining its product-authored tools.

## Wave 5: data lifecycle and integration contracts

Wave 5 starts with specifications and conformance. Runtime helpers follow only where the contract proves a stable owner.

### 22. Specify tombstone horizons and full resync

**Evidence:** Tiefgang and Eigenruhe implement finite tombstone retention with a stale-cursor response and client reset. Baukit's current offline contract does not define this lifecycle.

**First deliverable:** Extend `docs/platform/offline-readiness-contract.md` with:

- a monotonic per-owner purge horizon;
- cursor zero as an explicit full-rebuild request;
- a stable machine-readable stale-cursor response containing the horizon;
- preservation of pending mutations and explicit rejection records during local reset;
- atomic reset and cursor update;
- a stop condition if the server reports another stale cursor immediately after reset;
- parent, child, pull-only, and pending-edit cases; and
- a product hook that decides when a disruptive reset is safe.

**Second deliverable:** Add optional conformance callbacks and cases to `@baukit/sync-client/conformance`.

**Do not yet add:** generic table registration, dynamic deletion SQL, or a full sync engine. Prove the contract in a second product before deciding whether `baukit-sync` should own small PostgreSQL horizon helpers.

**Done when:** Tiefgang and one other finite-retention product pass the same reset cases.

### 23. Design optional durable-job ownership

**Evidence:** Tiefgang must find personal jobs by inspecting JSON payload keys during erasure.

**Design question:** Add an optional opaque `owner_key` or `partition_key` to the Baukit-owned job table, enqueue API, and indexes. The key has no prescribed identity format and can never be a metric label.

**Required design work:** additive migration, compatibility for existing rows, enqueue-in-transaction support, bounded cancellation or deletion by owner, behavior for running jobs, erasure ordering, index cost, and a second product example.

Do not combine this schema change with terminal cleanup. They solve different problems and have different compatibility risks.

**Done when:** a design note and two product mappings prove that explicit ownership replaces payload inspection without forcing a user schema into `baukit-jobs`.

### 24. Add a provider credential-probe contract

**Evidence:** Tiefgang's GitHub and Toggl adapters independently map credential checks to revoked, missing scope, rate limited, timeout, unavailable, invalid data, and an external account ID.

**Target:** a small port and fake in `baukit-integrations`, separate from paged import jobs.

**Contract:** return opaque account identity and connection health, preserve `Retry-After`, bound response handling, and keep provider text out of logs and public errors. Products own scopes, headers, endpoints, response parsing, and account models.

**Done when:** both Tiefgang adapters pass one conformance suite without provider branches in Baukit.

### 25. Add import-envelope conformance

**Evidence:** Eigenruhe has versioned, allowlisted, bounded, previewed, atomic import. Tiefgang's partial import shows why Baukit should define safety without claiming a shared data format.

**Target:** optional helpers in `@baukit/data-contracts` and, for backend imports, `baukit-test` only if a matching Rust consumer exists.

**Contract:** caller-supplied envelope decoder, field allowlist, preview planner, limits, and transaction adapter. Tests must prove that preview writes nothing, forbidden metadata never reaches product writes, commit is all-or-nothing, and cursor or sync state changes only after commit.

**Fixtures:** unknown fields, duplicate IDs, tombstones, ownership and revision fields, oversized strings, excess rows, mixed versions, and a failure halfway through commit.

Products keep schema versions, entity decoders, conflict policy, provenance, deletion order, and user copy.

### 26. Publish inbox and webhook reliability recipes

**Inbox:** Extend `docs/platform/integration-reliability.md` and optionally `baukit-test` with first delivery, exact replay, concurrent replay, rollback after inbox insert, domain failure, outbox failure, owner isolation, and durable outcome replay. Require an explicit idempotency scope such as owner plus source plus event ID. Do not copy Tiefgang's globally unique event ID choice.

**Webhook:** Document one job per subscription and event, receiver idempotency, signing input, rotation, retry classification, disable policy, and safe URL handling. Add signing and scripted-receiver test helpers. Do not extract Tiefgang's current handler because one retry can redeliver already successful targets.

A future `baukit-webhooks` crate requires two products with the same subscription and delivery model plus a complete server-side request-forgery policy.

### 27. Add raw OpenAPI mirrors only after a second consumer

**Evidence:** Tiefgang separately copies schemas for MCP and extension packages. Existing `openapi.consumers` correctly means generated TypeScript declarations, not byte copies.

**Design:** Add a distinct `openapi.mirrors` list if another product needs a packaged raw schema. Doctor and strict CI must verify byte equality, reject paths outside the product root, and keep declaration generation separate.

This item is optional convenience. It must not delay MCP generation, which can use the registered declaration.

### 28. Publish a live-row cap PostgreSQL recipe

**Evidence:** Eigenruhe has product SQL helpers for per-owner, per-parent, and per-day live-row caps. The underlying concurrency problem repeats, but its dynamic identifier helper is not a suitable Baukit API.

**Target:** Add a recipe under `docs/platform/` and concurrency helpers in `baukit-test`.

Compare row locking, serializable transactions, maintained counters, and database constraints. For each method, state how tombstones affect capacity, which index supports the check, how an update at capacity behaves, and how a product maps the rejection to its stable limit code.

The conformance helper should race two inserts at the last available slot and require exactly one accepted create. It should also prove that an update succeeds at capacity and a soft delete releases capacity.

Do not move dynamic table or column names into `baukit-sync`. Products continue to own their SQL, schema, scope, and cap values.

## Wave 6: bounded studies

Each study ends with one written decision: implement a package, add a contract or recipe, or keep the code duplicated. A study does not default to implementation.

### 29. Revisioned write queue and durable form drafts

Leitbild has web and mobile revision-aware autosave. Redemut has durable form drafts. Study them as related but separate concerns.

The possible write queue owns serialization, coalescing, acknowledged revision, cancellation, stale completion handling, conflicts, and reset on identity or document change. It stays framework-free. React hooks, request fields, and conflict copy remain local.

The possible draft helper owns a versioned decoded value, dirty state, save and clear operations, identity and document keys, and recovery after interruption. Form schema, UI, and submission remain local.

Do not merge server autosave and local draft persistence into one controller unless the combined interface is smaller than the two product compositions.

Required cases include edits during save, several queued saves, retry, conflict, unmount, account switch, document switch, corrupt draft, schema upgrade, submit success, and failed draft deletion.

### 30. Offline asset management

Use Eigenruhe as the source implementation and seek one second asset-heavy product. Study a neutral manager with injected manifest, byte stream, hash, metadata store, and file store plus optional Expo FileSystem and browser Cache Storage adapters.

The contract must cover queued, downloading, paused, complete, stale, corrupt, and failed states; cancellation; in-flight deduplication; hash-before-read; cleanup planning separate from deletion; identity change; and caller-owned fallback policy.

Keep content units, locale and voice metadata, storage copy, CDN policy, signed URLs, and playback integration local. Do not add media tools or provider SDKs to baseline projects.

### 31. Expo UI and headless accessibility behavior

The second-consumer condition in ADR 0001 is now met by several products, but their APIs have not been compared. Review Tiefgang, Eigenruhe, Redemut, and Fitness Tracker at the prop, state, and accessibility-test level.

Start with labeled fields, overlays, switches, segmented or roving choices, route-state views, chart table alternatives, context menus, safe-area helpers, sliders, toasts, and modal stacking.

Prefer additions to `@baukit/a11y-core` when only interaction state repeats. Create `@baukit/ui-expo` only when rendered components can accept product tokens, copy, and layout without a collection of product flags.

Test iOS and Android screen readers, web keyboard use, focus restoration, Escape and back behavior, reduced motion, large text, high contrast, disabled-only menus, action failure, routing, and stacked overlays.

### 32. Notifications and timeline playback

**Notifications:** Compare Eigenruhe and Redemut. Separate civil-time occurrence and finite-horizon reconciliation from reminder eligibility, quiet-hour policy, copy, channels, and deep links. Any Expo adapter must cancel only Baukit or feature-owned identifiers, never every scheduled notification.

**Timeline:** Seek a second timed-media product before extracting Eigenruhe's wall-clock runner. A possible core uses an immutable caller-owned timeline, monotonic clock, deterministic late-tick cue behavior, pause, resume, seek, completion, and serialized anchors. Audio, keep-awake, remote controls, cue types, and completion policy remain adapters or product code.

### 33. Calendar export

Compare maintained Rust and TypeScript iCalendar libraries first. If a dependency satisfies deterministic encoding, timezone conversion, UTF-8 folding, recurrence, and licensing requirements, document that choice rather than adding Baukit code.

If no suitable pair exists, define neutral events and one shared vector corpus before implementation. Resolve DST gaps and folds, line folding units, UID inputs, one-off versus recurring events, and deterministic ordering. Product titles, descriptions, routes, plans, and calendar selection remain local.

Native calendar adapters remain deferred until they report resumable per-item outcomes and define which records they may update or delete.

### 34. Release, GitOps, and migration compatibility

Start with a release manifest and a read-only validator. The manifest should describe process images, immutable source pins, target values files, and GitOps repository location. The first tool prints or writes a patch and supports dry run. It must work when desired state lives in a separate repository.

Do not port Leitbild's repository names, branches, registry owner, namespaces, cluster, retention counts, or push behavior. Any later mutation or pull-request creation needs exact target validation and explicit invocation.

Document expand-and-contract migration rules before automating them. Do not upstream the current regular-expression SQL scanner. A future gate should use reviewed migration metadata or a PostgreSQL-aware parser and must exercise N and N-1 application versions against the schema.

### 35. Browser identity composition

Redemut's account bootstrap and popup login solve real web composition problems, but both source implementations need a security correction before they can define Baukit behavior.

**Server-confirmed account bootstrap:** Specify states for an OIDC session, backend-confirmed account, unavailable backend, absent account, and blocked identity mismatch. A cached account may be reused only when it is bound to a trusted, successfully decoded subject. Undecodable or mismatched identity fails closed. Local repositories must not mount against a display claim or stale account key.

**Popup OIDC login:** Study an optional `@baukit/auth-web` coordinator with a fresh per-attempt correlation value, exact origin validation, one active attempt, timeout, popup-close detection, cleanup, and a full-page redirect fallback. UI, copy, route choice, and identity-provider policy stay local.

**Token management recipe:** Document the common personal-token posture without generating product scopes. Return plaintext once, require OIDC for list, create, and revoke, keep list and revoke owner-scoped, and prevent one personal token from minting another. A wrapper may map the verified token ID to product authorization context. Baukit should not add a generic scopes column until a second product proves the same data shape.

**Universal auth storage:** Prefer a composition example that selects secure native storage or browser storage over a new package. Any shared code must define logout cleanup, identity changes, proactive refresh ownership, and concurrent refresh behavior.

### 36. Other deferred contracts

Keep these items visible, but do not schedule package work until their stated evidence arrives:

| Candidate | Next evidence | Likely result |
| --- | --- | --- |
| Content-bundle manifest | A second non-learning content format | Generic manifest and atomic-install contract |
| Private local artifacts | A second blob or file use case | Retention and erasure conformance |
| Celebration arbitration | Direct comparison with Fitness Tracker | Headless queue behavior or keep local |
| Accessible context menu | A second component and edge-case tests | `@baukit/a11y-core` hook |
| Secret URL tokens | A second non-calendar use | Auth recipe or capability-link contract |
| SQLite migrations | A second raw-driver implementation | Conformance helper before any runner |
| Feature gates | Two products with stable runtime gates | Platform recipe and generated example |
| Browser extensions | A second Baukit extension | Optional quality template, not runtime logic |
| Sync coordinator | Callback comparison with another local-first app | Conformance or coordinator only if simpler than composition |
| Content build tools | A second generated-media pipeline | Optional tool with caller-owned metadata |
| LLM and speech adapters | Another product using the same provider contract | Provider package, while prompts and mappings stay local |
| Minimal runtime images | Reproducible in-container evidence on supported targets | Deployment recipe or supported image target |
| MCP runtime helpers | A second server with structured result and error needs | Small request-metadata, safe-error, and logging helpers |
| Network-first read cache | A second client with matching invalidation and empty-cache rules | Framework-free coordinator or recipe |
| Markdown editor commands | A settled product Markdown dialect and a second editor | Pure command helpers, never product UI or sanitizer policy |
| Word-count parity | One settled product definition used at every ingress | Cross-runtime fixture workflow, not a universal count rule |
| Web dialog component | A second implementation with matching native-dialog behavior | Headless behavior or a later web UI package |

## Work that stays product-owned

The following categories are outside this plan even when their implementations are tested and duplicated across runtimes:

- domain entities, calculations, state machines, scoring, progress, streaks, rewards, schedules, and content models;
- database tables, product migrations, repository methods, owner joins, explicit sync SQL, entity codecs, and conflict decisions;
- API paths, DTOs, scopes, token kinds, route groups, quotas, reason codes, and user-facing recovery choices;
- prompts, tool definitions, MCP descriptions, tool inputs, defaults, annotations, and returned private content;
- provider URLs, scopes, wire mappings, webhook destinations, health meanings, and external account models;
- navigation, layouts, colors, typography, icons, illustrations, animations, chart meanings, copy, and translations;
- extension permissions, blocking rules, native enforcement, calendar record policy, and operating-system integration choices;
- retention durations, deployment topology, environment state, image repositories, SLOs, release approvals, and rollback windows; and
- product metric names, dashboard panels, alert thresholds, analytics event names, and consent copy.

Code in these categories can supply fixtures and failure cases. It should not be moved into Baukit.

## Pull request sequence and gates

The following order minimizes rework while keeping reviews small:

| Order | Pull request | Depends on | Product adoption proof |
| ---: | --- | --- | --- |
| 1 | JSON rejection classification | None | Eigenruhe |
| 2 | Typed API-token store error | None | Leitbild |
| 3 | Principal-establishing middleware | None | Leitbild and Eigenruhe |
| 4 | Named route-group limits | 3 | Eigenruhe |
| 5 | Terminal job cleanup | None | Tiefgang |
| 6 | Recurring job slots | None | Eigenruhe |
| 7 | Runtime budget measurements | Placement decision | Eigenruhe |
| 8 | Sync overlapping-response cases | None | Tiefgang Dexie and SQLite |
| 9 | Browser sync environment | None | Redemut |
| 10 | Serialized preference mode | None | Redemut |
| 11 | Hybrid logical clock | Shared vector location | Redemut Rust and TypeScript |
| 12 | PWA worker export | Artifact-format decision | Eigenruhe |
| 13 | Browser QA backport | None | Generated auth and non-auth fixtures |
| 14 | Environment reconciliation | None | Generated fixture |
| 15 | Markdown link gate | None | Generated strict fixture |
| 16 | Keycloak policy validator | None | Generated auth fixture |
| 17 | Development realm reconciler | 16 | Real Keycloak fixture |
| 18 | Accessible Keycloak child theme | Existing theme decision | Real Keycloak browsers |
| 19 | Small locale and identity helpers | Independent API reviews | Redemut or Leitbild per helper |
| 20 | Node authentication package | Security review | Two MCP or CLI consumers |
| 21 | MCP generator | Bearer provider; Node auth optional | Generated fixture plus one product |
| 22 | Tombstone horizon contract | 8 preferred | Tiefgang plus one product |
| 23 onward | Wave 5 contracts and Wave 6 studies | As listed above | Per-item evidence record |

Do not bundle all Rust changes or all TypeScript changes into one release pull request. Each item changes a different contract and needs an independent adoption decision.

## Verification matrix

Run `make ci` for every implementation pull request. Add the following gates according to touched areas.

| Change | Additional local verification |
| --- | --- |
| Any version change | `scripts/check-version-coherence.py` |
| Rust dependency change | `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check advisories licenses` |
| New Rust language or standard-library use | `cargo +1.95 check --manifest-path rust/Cargo.toml --workspace --all-targets` |
| PostgreSQL jobs, auth adapters, or sync helpers | `cargo test --manifest-path rust/Cargo.toml -- --include-ignored` |
| Sync-client or Dexie contract | `make ts-browser-test` |
| Expo SQLite contract | `make expo-sqlite-conformance` |
| Mobile templates or dependencies | `make native-android-gate` |
| CLI or template change | bless snapshots, inspect their diff, then run the full generated fixture for every affected flavor |
| Keycloak assets or reconciliation | pinned-container unit and real-browser suite for every supported patch |
| PWA package | test the packed artifact and generated worker build |
| Observability or metric names | `python3 deploy/observability/lint/check-metric-names.py` |

For CLI, template, or public API work used by templates, run the generated fixture sequence documented in `CLAUDE.md`. A capability that is optional must have both a selected fixture and an unselected fixture proving that it leaves no dead dependency or configuration.

## Completion criteria

An item is complete only when all of the following are true:

- the Baukit contract names no source product concept;
- unit, integration, conformance, and artifact tests appropriate to the risk pass;
- README, changelog, platform docs, and migration notes are current;
- generated output contains the feature only when selected;
- logs, metrics, errors, fakes, and test failures do not expose credentials or private product text;
- at least one named source product adopts a released Baukit version;
- the adopted product deletes the replaced local mechanism; and
- CI-equivalent checks pass in Baukit and the adopting product.

The plan has failed if Baukit gains a copy while every application keeps its fork. The measurable result is less product-owned platform glue, backed by the same or stronger failure tests.
