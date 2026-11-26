# F: correctness and verification plan for baukit's critical components

Scope: every baukit crate, package, template and workflow that decides who a caller is, what they may do,
whether user data stays consistent and isolated, or what ships to the 12 products. Read-only analysis on main at
ae19c9cf (2026-11-25), plus an implementation plan. No source files changed.

Method: two rounds of four parallel reviews.

- Round 1 covered auth, the sync building blocks, baukit-suite's signed protocols, and the crypto and
  request-boundary primitives.
- Round 2 covered client-side data isolation, erasure, push and migrations, the code the templates generate, and
  organization roles, api-runtime and the release pipeline.

I re-read the top findings of every review in the code before accepting them. Status in the tables:

- **verified**: re-read by hand and the failure path confirmed.
- **reported**: one review traced it with file and line, not yet re-read.
- **suspected**: depends on how a product wires it, or on behaviour outside this repo.

Every fix task starts by re-checking its finding.

## 1. Why this exists

baukit already has 7 cargo-fuzz targets (60 s each in the `fuzz` lane), proptest in 4 crates, and a threat model
with 20 covered rows. That catches parser panics. It does not catch the bugs below, which are logic and wiring
errors in code that parses fine.

The worst one: every generated API starts its listener without `ConnectInfo`. The rate limiter never sees a client
address, so all clients of a product share one IP bucket. Every test inserts `ConnectInfo` by hand, so none of
them see it. Bugs like that ship into all 12 products on the next version bump, and they are exactly what spec-level
checking and composed-system tests find.

The plan adds four things on top of what exists:

1. Fixes for the confirmed bugs, each landing with the test that would have caught it.
2. A conformance gate for generated products. It tests the router a product actually composes, over a real
   socket, instead of the pieces in isolation.
3. Machine-checked specs:
   - Kani proves small pure kernels correct for every input within stated bounds.
   - Verus proves three of them with no bounds.
   - TLA+ models check the protocols that span processes, transactions, tabs and devices, where no code-level
     prover can see the bug.
4. Fuzz targets and property tests with real oracles. Rust-vs-TypeScript and Rust-vs-njs differential tests keep
   the two halves of each protocol from drifting.

## 2. Critical components

Ranked by blast radius: how many requests or users pass through the code, and what an attacker or a bug gets when
it fails.

### Tier 1: every request, every user, every byte of user data

| Component | Code | What it decides |
|---|---|---|
| Generated server wiring | `rust/crates/baukit-runtime/src/lib.rs` (listeners), `templates/backend/**/bin/api.rs`, `rate_limits.rs` | whether the limiter, auth and body limits actually cover every route of every product |
| JWT verification and JWKS cache | `rust/crates/baukit-auth/src/verifier.rs` | whether a bearer token is a user |
| BFF browser session | `rust/crates/baukit-auth/src/browser_session/` | cookie to access token, refresh rotation, reuse detection |
| API tokens | `rust/crates/baukit-auth/src/api_token.rs`, `api_token/format.rs` | machine credentials, revocation |
| Rate limiter | `rust/crates/baukit-ratelimit/src/` (XFF, buckets, Redis scripts) | the only DoS control in front of every route |
| Client account isolation | `typescript/packages/data-contracts/src/identity.ts`, `scoped-lifecycle.ts`, `erasure.ts`, `web-storage.ts` | on a shared device, whether user B can ever see, sync or lose user A's data |
| Client API runtime | `typescript/packages/api-runtime/src/fetch.ts` | where the access token is sent, and which responses can sign a user out or wipe local data |
| Sync building blocks | `rust/crates/baukit-sync/src/` (HLC, revisions, purge horizon), `typescript/packages/sync-client`, `data-contracts` | ordering and convergence of all offline-first user data |
| GDPR erasure | `rust/crates/baukit-erasure/src/`, `baukit-data-subject/src/server` | whether an erased user is really gone and stays gone |
| Job leases | `rust/crates/baukit-jobs/src/store.rs`, `runner.rs` | one effective attempt per job (erasures, emails, suite deliveries) |
| Egress policy | `rust/crates/baukit-egress/src/policy.rs`, `client.rs` | SSRF into the cluster network |

### Tier 2: signed protocols, authorization data, release

| Component | Code | What it decides |
|---|---|---|
| Suite signatures, inbox, tickets, link flow, entitlements | `rust/crates/baukit-suite/src/`, `typescript/packages/suite-client` | which product events and plan tiers are trusted |
| Suite organizations and credits | `baukit-suite/src/platform/organizations.rs`, `plan_organization.rs`, `credits.rs` | who acts for an organization, who pays for what |
| Release pipeline | `.github/workflows/release.yml` | what code gets published to crates.io and npm under baukit's name |
| Push device registry | `rust/crates/baukit-push/src/` | whose phone gets whose notifications |
| Schema migrations | `rust/crates/*/migrations`, `*_MIGRATION_SQL` | whether an upgrade can lock or break a live product database |
| Telemetry redaction | `rust/crates/baukit-telemetry/src/lib.rs` | whether secrets reach the OTLP collector |
| `baukit-test` security helpers | `rust/crates/baukit-test/src/auth_negative.rs`, `cross_tenant.rs` | whether a product's security tests prove anything |
| OAuth state, webhooks, media grants, vault, MCP | see section 3 | callback CSRF, signed inbound events, media access, secrets at rest, tool scopes |

### Tier 3: bounded parsers, receipts, client purchase state

`baukit-data-subject` receipts, `baukit-events` envelopes, `baukit-http` header parsers, data import and CSV
export, `purchases-core`. These fail closed or are already vector-tested. They get oracle upgrades and the findings
below, not new machinery.

## 3. Findings

Severity is for a product deployed as the templates generate it. The "catches it" column names the technique in
section 4 that would have found the bug, so each fix also shows the technique is worth its cost. IDs are stable;
the tables are ordered by severity, not by ID.

### 3.1 High

| ID | Status | Finding | Failure scenario | Fix | Catches it |
|---|---|---|---|---|---|
| F-36 | verified | Generated APIs start through `serve_listener_pair`, which calls `axum::serve(listener, router)` without `into_make_service_with_connect_info` (`baukit-runtime/src/lib.rs:761`). Nothing in `rust/` or `templates/` adds `ConnectInfo`. With no peer, `resolve_client_ip` returns `None` and never reads XFF, so the key is `rl:ip:unknown` (`ratelimit/src/options.rs:93`). The browser-session and suite public limiters read the same missing extension. | One client spends the fleet-wide IP budget (6000/min + 500 burst) and every user of the product gets 429, authenticated users included. `/auth/session` login is capped product-wide. Every unit test inserts `ConnectInfo` by hand (`axum_layer.rs:746`, `ratelimit/tests/redis.rs:368`, `auth/tests/browser_session.rs:502`), so none sees it. | Serve with `into_make_service_with_connect_info::<SocketAddr>()` in `spawn_listener`. Make a missing peer a startup or request error in the limiter, not a shared bucket. | conformance gate G-02 (real socket) |
| F-01 | verified | `resolve_client_ip` returns the socket peer as soon as any XFF entry fails to parse, including entries the client wrote (`ratelimit/src/axum_layer.rs:297-306`). The test `xff_resolution_honors_trusted_hop_count` asserts this fallback. F-36 masks it today, and fixing F-36 exposes it. | Behind one proxy, a client sends `X-Forwarded-For: x`. Its bucket becomes the proxy's IP, shared with everyone who falls back. It alternates headers to double its quota, or drains the shared bucket. | Parse only the entry at `len - hops`. Never touch entries to its left. An unparseable trusted entry is a proxy misconfiguration: log it and use the peer. | Kani K-01 |
| F-02 | verified | `RateLimitConfig::default` sets `trusted_proxy_hops: 1` (`baukit-config/src/lib.rs:260`). | A product run without a proxy trusts the client's XFF, so any client picks any bucket, including a victim's IP. | Default to 0. The chart and templates set it from the ingress topology. Validation fails when hops > 0 and no proxy CIDR is configured. | config test |
| F-03 | verified | An unknown `kid` forces a JWKS refetch even when the cache is fresh. The global `tokio::Mutex` stays held across the HTTP fetch (`auth/src/verifier.rs`, `key_for`). The negative cache is per kid, so random kids bypass it. | Unauthenticated requests with `{"alg":"RS256","kid":"<random>"}` and any signature cost one JWKS GET each. Valid verifications queue behind them. The IdP throttles the product and auth answers 503 for everyone. | Global forced-refresh cooldown (one unknown-kid refetch per 30 s). Verify known kids against an `ArcSwap` snapshot with no lock. Single-flight the refetch without blocking readers. | proptest P-03, Kani K-05, gate G-04 |
| F-37 | verified | `establish_principal` is the outermost layer on item routes (`templates/backend/.../rate_limits.rs:56-60`). It verifies and rejects bearers before any limiter runs (`auth/src/axum_integration.rs:52-64`). | F-03's amplification runs with no rate limit in front of it. Junk tokens are free for the attacker and cost the product one verify each. | Run a cheap IP pre-limit outside `establish_principal`, and count failed verifications against the IP bucket. Keep the identity limiter inside. | gate G-04 |
| F-38 | verified | `ScopedPersistenceLifecycle.handleProfileErased()` takes no subject and erases `lastSelectedPartition` at the time it runs (`data-contracts/src/scoped-lifecycle.ts:130-140`). api-runtime fires `onProfileErased` for any 401 `profile_erased` with no session check (`api-runtime/src/fetch.ts:155-160`). The template calls `clearSession`, which broadcasts to every tab. `identity.test.ts:395` locks this in. | A's profile is erased on another device while a request with A's token is in flight. A signs out, B signs in, and B's partition is ready. The stale 401 lands and deletes B's partition, unsynced outbox included, and B is blocked as `profile-erased`. | `handleProfileErased(subject)` erases only that subject's partition. api-runtime stamps each request with the session generation and drops signals from an older one. Same for `handleSessionExpired` and `blockIdentityMismatch`. | TLA+ M-07, fast-check C-02 |
| F-40 | verified | The Clerk/WorkOS deleters and the Keycloak deleter treat any 404 as "account deleted" (`erasure/src/api.rs:63`, `keycloak.rs:139`). | A wrong `base_url` (for example missing `/v1`) or a subject that isn't the provider's user id returns 404 on every call. The operation goes `completed`, the fence relaxes for fresh logins, and the IdP account still exists. The signed receipt claims the user was erased. | Accept 404 only when the provider's error body says "user not found". Probe the configured base URL at startup. | TLA+ M-06 (404 as a nondeterministic answer), fuzz Z-12 |
| F-39 | verified | Organization membership is cached in a private per-process `HashMap` for 60 s (`suite/src/platform/plan_organization.rs:120-125`). No public API evicts it, so products cannot wire `organization.membership_changed` into it. `is_member` drops `account_role`, so a Billing-role member gets the full organization grant. | An admin removes a member. The member keeps organization access for up to 60 s on every replica, and gets a fresh 60 s on any replica that hadn't cached them yet. Role-based gating is left to each product, and none get a helper. | Public `evict(org, subject)`, wired to the membership and seat events in the template. Return the role from `is_member` and add `require_role`. Injectable clock. | TLA+ M-08, proptest P-15 |
| F-05 | verified | The job lease predicate is `locked_by = $2 AND locked_until > $3` (`jobs/src/store.rs:35`). All concurrency slots of a runner share one `worker_id`. | Attempt 1 stalls and its lease expires. A sibling slot reclaims the job as attempt 2. Attempt 1 wakes up and calls `complete`, which still matches, so the job is marked succeeded while attempt 2 runs. | Bind the attempt number (or a random lease id) into every terminal write's predicate. | TLA+ M-02, proptest P-10 |
| F-04 | verified | HLC `observe` accepts any remote wall time up to the 2^53 limit (`sync/src/hlc.rs:179-200`, `sync-client` `hlc.ts:142-158`). No drift bound exists. | A device with its clock set to 2100 pushes once. Every device that pulls jumps to 2100 and wins last-writer-wins against honest edits for decades. A remote at `2^53-1` makes every observer's next `now()` fail, and since that state is persisted, the device can never write again. | `observe` takes a `max_drift_ms` and rejects remotes beyond `physical + max_drift`. The server checks client HLCs on push against its own clock. Same change in TS. | Kani K-09, Verus V-01, differential D-01 |
| F-60 | reported | `baukit-test` helpers that products use to prove auth fails closed are weak. `auth_negative` probes only operations that declare `security`, so an accidentally public route is skipped, and it has no positive control, so a verifier that rejects everything passes. `cross_tenant` skips operations without path params, has no owner positive control, and never re-reads after a denied write. Template conformance tests target the api crate's `router()`, not the composed router. | A product ships a route that is accidentally public, or a cross-tenant leak on a list endpoint, and its "security tests" stay green. | Require `.public(method, path, reason)` for every unsecured operation and fail on anything unlisted. Positive controls in both helpers. Re-read after denied mutations. Run them against the composed router. | gate G-01, G-05 |
| F-06 | reported, plausible | Pull pagination with `revision > cursor LIMIT n` loses rows when one revision stamps several rows. The crate's own example stamps one revision on every row of a push (`sync/src/lib.rs:65-67`). The README pull uses `LIMIT 100`. `(owner_id, revision)` is not unique (migration 0001:31). | A push stamps rev 7 on rows A and B. Page size 1 returns A, the cursor becomes 7, and `> 7` skips B forever. | A pull helper with a `(revision, id)` tuple cursor, or one revision per row plus the unique index. Fix the README. | TLA+ M-01 |
| F-07 | reported, plausible | A multi-statement pull under READ COMMITTED can skip rows (`sync/src/purge.rs:233-237` only says "same transaction"). | The pull reads table 1 (> 5). Writer T commits rev 6 in table 1 and rev 7 in table 2. The pull reads table 2, sees 7, and the client stores cursor 7. Table 1 at rev 6 is lost. | `guard_pull_cursor` reads the high-water mark first and returns it, and every table query filters `revision <= hi`. Or the guard asserts REPEATABLE READ. | TLA+ M-01 |

### 3.2 Medium

Auth and session:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-11 | verified | BFF refresh rotates the refresh token at the IdP inside an open DB transaction (`browser_session/service.rs:191`). A client disconnect drops the future, or the commit fails, and the old token stays stored. With IdP reuse detection (Keycloak, Auth0, Okta), the next refresh revokes the family and logs the user out. auth-node (`device-flow.ts:334-375`) and auth-native (`index.ts:390-424`) have the same shape. | Run the IdP call and the store in a detached task that survives cancellation. On commit failure, retry the store with the new token, never the old one. |
| F-12 | verified | No stale-on-error for JWKS. After `cache_ttl`, even a known kid needs a successful fetch. `JwkSet` parsing is all-or-nothing (`verifier.rs:777-803`), so one key with a missing `kty` fails every token. | Serve known kids from a stale set for a bounded grace while the IdP is down. Skip unparseable keys and log them. |
| F-13 | reported | auth-web `isFresh` compares the server's absolute `expiresAt` with the client clock (`browser-session-client.ts:375`). A client clock ahead makes every call refresh into the 12/min bucket; a clock behind keeps sending expired tokens. | Server sends `expiresIn`. Client computes expiry from its own receipt time. |
| F-41 | verified | api-runtime attaches `Authorization: Bearer` to any URL it is given (`fetch.ts:259-301`). `//evil.example/x`, `/\evil.example` and absolute URLs leave the base origin, and `Request` inputs skip resolution. A foreign 401 triggers a forced token refresh and replays the fresh token to that origin (`fetch.ts:204-224`). A foreign 401 with `profile_erased` signs the user out and wipes local data (with F-38, possibly another user's). A leading `/` drops the base path prefix. No baukit caller passes server-derived URLs today, so this is latent. | Attach the token and honour 401 signals only when the request origin equals the base origin and the path is under the base path. |

Rate limiting and server wiring:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-51 | reported | In the auth flavor, item routes run with no limiter when both scopes are disabled (`api.rs:189`, `redis_store.rs:160`), and `RateLimitConfig::validate` accepts that outside local. `fail_mode` defaults to `Open`, so a Redis outage turns off all limiting. | Validation requires limiting outside local. Fail closed for the IP pre-limit (F-37), configurable for the identity limiter. |
| F-52 | suspected | The Traefik Service is a `LoadBalancer` without `externalTrafficPolicy: Local` (`deploy/platform/traefik/helm-release.yaml:45-48`). On more than one node, kube-proxy SNAT makes the peer a node IP, so with hops=1 clients are bucketed per node. | `externalTrafficPolicy: Local`, or PROXY protocol, plus a platform e2e test with two source IPs. |
| F-14 | verified | Job runner: `result??` propagates one attempt's `LeaseLost`, store error or `add_duration` overflow out of `run()` (`runner.rs:222, 250, 258, 276`) and aborts all in-flight attempts. `retry_after` from a vendor header is not capped at `retry_max`. | Log and continue per attempt. Clamp `retry_after`. Checked arithmetic. |
| F-15 | reported | In-memory store eviction can evict a drained bucket, which comes back full (`ratelimit/src/memory.rs:74-86`). Eviction is an O(n) scan under a global mutex. | Evict only full (idle) buckets, or use a TTL wheel. |
| F-16 | verified | Amount `release` uses the window at release time, not the window of the consume (`amount_budget.rs:296-312`). Consume at 23:59:59, release at 00:00:01, and the new window over-admits. | Return the window id from consume and release against it. |
| F-17 | verified | IPv6 clients are bucketed per /128 (`ratelimit/src/options.rs:90-95`), so one host has 2^64 buckets. | Bucket IPv6 by /64 (configurable). |
| F-22 | suspected | Redis Sentinel retry re-runs non-idempotent consume and release scripts after a lost reply (`redis_store.rs:443-525`). | Idempotency token per call inside the script, or no retry after the request was written. |
| F-50 | reported | OTLP export uses the raw `tracing_opentelemetry::layer()` (`telemetry/src/lib.rs:326-329`). Redaction only runs in the stdout JSON visitor. The sensitive-name list lacks `secret`, `key`, `code`, `session` and `credential`. | One redaction pass that both exporters share. Extend the list. A test that exports a span with each sensitive name to an in-memory OTLP sink. |

Erasure, push and data subject:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-42 | verified | `reject_fenced_subject` reads the fence with no lock (`erasure/src/store.rs:282-292`). The docs steer handlers to it rather than `guard_subject`. | A request passes the fence, `erase` commits, and the request then inserts a "get-or-create" profile or a push registration. Those rows survive the erasure. Fix: product writes run `guard_subject` inside their transaction. A template helper makes that the default. |
| F-43 | verified | Push tokens are the primary key, and registering a token another owner holds moves it (`push/src/postgres.rs:20-23, 563-571`). | Anyone who learns a victim's Expo token takes it over. The victim stops getting notifications and receives the attacker's. Erasing the attacker deletes the victim's row. Fix: a per-install secret registered alongside the token, required to move it. |
| F-44 | verified | The Expo sender uses `?` per chunk (`push/src/expo.rs:294-295`). | If chunk N fails, the outcomes of chunks 1 to N-1, which Expo accepted, are dropped. The documented "release the claim on failure" pattern then resends to those recipients. Fix: return per-chunk outcomes and a partial error. |
| F-45 | suspected | Data-subject `authorize` checks an allowlisted `client_id`, scope and "not an API token" (`data-subject/src/server/router.rs:69-82`), but not that the token came from a client-credentials grant. | If an allowlisted client also serves user logins, or users can request the scope, any such user can erase arbitrary subjects. Fix: require a client-credentials marker (no `sub` other than the client, or a `gty` claim) and test it. |

Client data and purchases:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-53 | suspected | `recheckServerSubjectBeforeSyncAdoption` checks the subject and then runs `adopt()` with no fence (`identity.ts:347-357`). A sign-in as B in another tab replaces tab 1's token mid-adopt, so A's outbox can go out with B's token. | Bind adopt to the lifecycle revision and an AbortSignal; the token provider refuses a token for another subject. |
| F-54 | reported | Client erasure has no durable tombstone between server success and local delete (`erasure.ts:126-146`), and the erased flag lives in memory. Pre-erase hook failures (stop sync, flush) are only warnings (`erasure.ts:115-120`). | A crash after the server erase leaves the partition, which reopens and syncs on reload. Fix: a durable erased marker written before the server call and cleared after the local delete; a failed pre-erase hook aborts. |
| F-55 | verified | `WebStorageKeyValueStore` builds keys as `namespace + key` with no delimiter (`web-storage.ts:30-58`). | With namespace `drafts:u1`, `clear()` deletes `drafts:u10…`. Fix: length-prefix or a reserved delimiter, plus a migration for existing keys. |
| F-56 | reported | The registry lock is an in-process WeakMap keyed by the adapter object (`identity.ts:244-256`), and the web template creates a new registry per render. | Concurrent tabs lose registry updates: an erase removal is overwritten and orphan cleanup misses stores. Fix: Web Locks on web, compare-and-set on the stored document. |
| F-57 | suspected | Client purchase state can widen access: `#accept` ignores a `null` answer (`purchases-core/src/flow.ts:265`) although the backend returns `null` for `suite_identity_missing`; a lower revision is ignored after a server reset; `useOrganizationPlan` is not keyed by account. | A downgraded or switched user keeps the previous grant in the UI. Fix: `null` clears the grant, the revision is scoped to the subject, the org plan is keyed by account and checked against the returned ids. |

Sync, suite and egress:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-08 | verified | `HubAccountPlan::refresh` returns `free()` on `NoEntitlements` but never stores it (`suite/src/platform/plan.rs:391-424`). `current()` keeps serving the cached paid snapshot until ticket `exp`, which can be 7 days. | Store the free snapshot with a revision bump. Test that `current()` is free after a refresh says so. |
| F-09 | verified | `rankPushBatch` keys entities as `` `${entityType}:${entityId}` `` (`sync-client/src/push-batch.ts:89`). `("a:b","c")` and `("a","b:c")` collide, so one change is dropped and its changeId settles with the other entity's outcome. | Nested `Map`, or `JSON.stringify([type, id])`. |
| F-10 | verified | Suite return URLs accept `<own_scheme>-dev` in every environment (`suite/src/domain/mod.rs:427-432`). A malicious app that registers it on a prod device receives `state`, `request` and `code`. PKCE stops redemption, so the impact is a hijacked or stuck flow. | Gate behind a config flag that is off in prod. Reject peer schemes `http`, `https`, `javascript`, and collisions such as peer B's scheme being `A-dev`. |
| F-18 | verified | `x-suite-replay` is parsed (`suite/src/services/ingest.rs:68-72`) but is not in the signing input. An on-path proxy can strip it within the 300 s window, which turns a historical replay into a live event that grants rewards. | Move the flag into the signed body, or into a v2 signing input. |
| F-19 | reported | Multi-tab web HLC: two tabs share a deviceId, read storage once and serialize per instance (`hlc.ts:126, 179`). They emit identical timestamps and overwrite each other's state. | Web Locks around read-advance-write, or a per-tab device suffix. |
| F-20 | reported | Rust `validate_event_envelope` skips rules the TS validator enforces: payload key cap and regex, `sourceApp` regex, `userId` bounds, Z-only timestamps (`events/src/lib.rs:113-136` vs `events/src/index.ts:36-56`). A far-future `occurredAt` is always accepted. | One spec enforced in both, shared vectors per rule, and a future bound. |
| F-21 | reported, design | Egress `OnlyEndpoint` accepts a private or loopback answer for the endpoint host (`egress/src/policy.rs:15`). If a tenant controls that DNS name, it's SSRF. | Separate "operator-configured internal endpoint" from "tenant URL" in the type. Only the former may resolve private. |
| F-47 | verified | Standalone `is_member` always returns `true` (`suite/src/platform/plan.rs:212-219`). | Any authenticated user sends any `X-Organization-Id` and gets the configured grant. A product that uses `require_feature(Member)` as its membership check is open in standalone mode. Fix: standalone mode requires a product-supplied membership port; no default `true`. |
| F-48 | suspected | Credits `reserve` with a repeated key returns the first reservation without checking it is still `Pending` (`suite/src/platform/credits.rs:252-258`). `settle` and `release` don't check the returned status or `charged <= amount`. | If a product builds the key from a client `Idempotency-Key`, a replay after settle runs the action again with no new charge. Fix: return `AlreadySettled`, validate hub responses. |
| F-49 | suspected | suite-client checks only that `authorizeUrl` from the server is a string (`suite-client/src/responses.ts:122`) before navigating a popup or native session to it. | A `javascript:` URL in an `about:blank` popup runs in the opener's origin. Fix: https only, origin allowlist. |

Release and migrations:

| ID | Status | Finding | Fix |
|---|---|---|---|
| F-46 | verified | `publish-crates` and `publish-npm` hold `id-token: write`, have no `environment:`, and fire on any `v*.*.*` tag on any commit (`release.yml:14-16, 163-201`). Nothing checks that the tag is on `main`. `pnpm install` and `pnpm build` run inside the OIDC job. | Anyone with push access to tags, or a compromised token, publishes a backdoored baukit that 12 products consume. Fix: a protected `release` environment with required reviewers, a `merge-base --is-ancestor` check, and build in an unprivileged job that hands artifacts to a publish-only job. |
| F-58 | reported | baukit's shipped migrations are not safe on a live database: push 0004 deletes rows and adds a `NOT NULL` column with no default; jobs 0002 adds validated CHECKs on `job_outbox` under ACCESS EXCLUSIVE, and old binaries then violate them; jobs 0003 rebuilds an index without `CONCURRENTLY` on the hottest table; triggers on `job_outbox` take SHARE ROW EXCLUSIVE with no `lock_timeout`; recommended owner FKs are added validated. No migration is re-runnable, and cross-crate ordering lives only in doc comments. | Expand-and-contract for column changes, `CONCURRENTLY` for indexes, `NOT VALID` then `VALIDATE` for constraints, `SET lock_timeout` in every migration, and a migration linter and old-binary test in CI (G-06). |

### 3.3 Low

- F-23 The MCP scope check skips non-JSON bodies and forwards them (`mcp/src/security.rs:293`). Reject with 400.
- F-24 The MCP quota key `mcp:{resource}:{issuer}:{subject}` is ambiguous because issuers contain `:` (`security.rs:235`). Length-prefix or hash the parts.
- F-25 With several audiences and no `allowed_clients`, any `azp` string passes (`verifier.rs:1114-1120`).
  Resolution: by design for access tokens. Audience is the trust boundary (RFC 9068), and client restriction is opt-in through `with_allowed_clients`. Generated `config/local.toml` sets `auth.authorized_parties` to the realm clients.
- F-26 Access-token verification never checks `typ`. If audience equals client_id, an ID token passes `verify()`. Add an opt-in `typ: at+jwt` requirement, and reject tokens that carry `nonce` on the access path.
- F-27 BFF `exchange` leaves the previous session family alive, and `logout` with an older rotated cookie deletes the whole family.
- F-28 auth-node never recovers a stale `tokens.json.lock` after SIGKILL.
- F-29 Jobs leases use the worker clock instead of DB `now()` (`runner.rs:217, 359`).
- F-30 Vault decrypt accepts any non-empty subset of fields, and an old ciphertext for the same (scope, field, version) swaps in silently.
- F-31 Receipt payloads accept unknown fields (`data-subject/src/types.rs:272`).
- F-32 The peer inbox hash covers only the payload (`ingest.rs:405`), unlike the hub inbox.
- F-33 Ticket parsing differs between Rust and TS: duplicate keys, `1.0` and `1e9` numbers, `manageUrl`, kid bounds.
- F-34 Misc boundary items:
  - Azure WireServer `168.63.129.16` is missing from the metadata denylist.
  - `Duration::from_secs_f64` can panic on a huge period.
  - MCP template `{x}` matches `..`.
  - Invalid bearers are not rate limited at MCP.
  - Ingest accepts zero-padded timestamps.
  - An ETag prefix ending in a digit is ambiguous.
  - The njs `KEY_ID_PATTERN` may accept a trailing `\n` that Rust rejects.
- F-35 Sync edge cases:
  - Foreign or corrupt HLC state resets to wall 0.
  - Owner erasure resets the revision counter, so stale cursors pull nothing.
  - `validatePushOutcomeCoverage` ignores extra outcomes.
  - The README example binds `owner_id` to `id = $2` (`baukit-sync/README.md:86-88`).
- F-59 Erasure and push edge cases:
  - The fence compares IdP `auth_time` with DB `now()`, so IdP clock skew lets recent sessions through.
  - The subject hash key has no versioning, so rotating it drops every fence.
  - Keycloak `allow_local_http` allows HTTP to any host, not just loopback (`keycloak.rs:58`).
  - `purge_expired` can delete pending erasures when retention is short (`data-subject/src/store.rs:247`).
  - A failed operation returns HTTP 200 (`erasure/src/store.rs:58`).
  - A login during a pending Delete-mode erasure keeps a valid token after completion.
  - Push claim `release` is not fenced to the claimer.
- F-61 Generated-wiring edge cases:
  - `parse_origins` accepts `null`, `http://` in production, and origins with paths (`baukit-http/src/options.rs:348-361`).
  - The chart schema allows `opsService.type` NodePort and LoadBalancer.
  - `X-Request-ID` is accepted with no length or charset limit and echoed into logs (`middleware.rs:85-96`).
- F-62 `baukit check` can be bypassed:
  - The walk skips any directory named `vendor` and honours `.gitignore` (`cli/src/check/walk.rs:8-28`).
  - The `#[allow(` matcher misses `#[cfg_attr(all(), allow(..))]`, `# [allow`, and attributes after code on the same line (`comments.rs:213-221`).
- F-63 Client data edge cases:
  - `commitImportEnvelope` reports failure after it committed when `afterCommit` throws, so a retry imports twice. A preview prepared under A can be committed into B.
  - `assertBoundedStrings` has no depth limit and throws `RangeError` on deep input.
  - TS and Rust CSV export encode native numbers differently (`1e21`, `1e-7`, `-0`), and full-width formula triggers are not neutralized.
  - The mobile SecureStore registry can exceed the ~2 KB item limit at about 15 profiles.
- F-64 Suite and client navigation:
  - Organization cache keys share the subject namespace (`plan_organization.rs:114-116`).
  - `normalizeResponseError` reads error bodies with no size limit.
  - Navigation hrefs from product config get no scheme check.

### 3.4 Threat-model drift and new rows

- S-01, S-05, S-06 and S-11 are closed in code but still read "gap".
- B-02 cites a test name that no longer exists, and B-01's test moved to `verifier/tests.rs`.
- B-08 cites the XFF test that encodes F-01, and claims coverage that F-36 voids.

New boundaries the model lacks:

- the client device as a multi-user store (F-38, F-53 to F-56);
- the release pipeline as a trust boundary (F-46);
- push token registration (F-43);
- the data-subject machine API (F-45);
- the OTLP collector as a data sink (F-50).

The first PR fixes the drift and adds rows for every high finding.

## 4. Verification strategy

### 4.1 Which tool for which property

| Tool | Proves | Use it for | Cost |
|---|---|---|---|
| Kani | A property for every input up to stated bounds; panic and overflow freedom with no annotations | Pure kernels: IP classification, XFF selection, expiry, skew and window arithmetic, HLC steps, canonical encoders, scheme and URL classifiers | Low. A harness is 10 to 40 lines. |
| Verus | A property for every input, no bounds, by SMT proof | Kernels whose bugs only show over long sequences: HLC monotonicity, token bucket admission bound, cursor progress | High. Expect about 5 lines of proof per line of code, so 3 kernels. |
| TLA+ with TLC | A protocol invariant over every interleaving of a small model | Anything spread across processes, transactions, tabs or devices: sync pull, job leases, BFF session family, suite link and inbox, erasure saga, client account switch, membership caches | Medium. 150 to 300 lines per model. |
| Composed-router conformance | The router a product actually runs, over a real socket | Layer coverage, layer order, peer address, security headers on every route | Low once the harness exists |
| proptest, proptest-state-machine | A property on random inputs; a real implementation against a reference model | Postgres and Redis stores, caches, registries, anything with I/O | Low to medium |
| cargo-fuzz | No panic, plus an oracle, on adversarial bytes | Every decoder of untrusted input | Low |
| fast-check, including `fc.commands` and `fc.scheduler` | proptest and model-based testing in TypeScript, with controlled async interleavings | api-runtime, data-contracts lifecycle and registry, sync-client, auth-web, suite-client, purchases | Low to medium |
| Differential vectors | Rust and TS (or njs) agree on every generated case | HLC, events, tickets, media grants, return URLs, CSV, limits | Low once the generator exists |
| Static gates | A pattern never appears | `axum::serve` without connect info, unwrapped `.merge`, unsafe migrations, workflow shape | Low |

Creusot and Aeneas are left out. Creusot covers the same ground as Verus here with less tooling around it. Aeneas
would mean proving Lean theorems about a library this size, which costs more than this job needs.

Two limits apply to all of it:

- **A proof only shows the code matches its spec.** Every spec gets a review against the invariant it claims to
  capture, and sits next to the code it proves.
- **Kani's guarantee holds only within its harness bounds** (`#[kani::unwind]`, input lengths). Each harness states
  its bounds in its doc comment, and section 6 publishes them.

The F-36 lesson applies too. Proving each piece correct says nothing about whether the pieces are connected, so
every proven kernel also needs a composed-system test that exercises it through the real wiring.

### 4.2 Pure kernels

Kani and Verus need code that takes everything as an argument: no `Instant::now()`, no I/O, no locks. Several
decisions are currently written inline with a clock read or a mutex. Each one moves into a small pure function in
the same crate, and the existing code calls it. This is a refactor with no behaviour change, and existing tests
must stay green across it.

| Kernel | Extracted from | Signature sketch |
|---|---|---|
| `select_forwarded_ip` | `ratelimit/src/axum_layer.rs::resolve_client_ip` | `(entries: &[&str], peer: Option<IpAddr>, hops: usize) -> Option<IpAddr>` |
| `ip_bucket` | `ratelimit/src/options.rs::ip_key` | `(ip: IpAddr, v6_prefix: u8) -> IpBucket` |
| `claims_time_valid` | `auth/src/verifier.rs` (`now` read at about :1010) | `(exp: Option<i64>, nbf: Option<i64>, now: i64, skew: u64) -> Result<(), TimeError>` |
| `jwks_refresh_allowed` | `auth/src/verifier.rs::key_for` (new, F-03) | `(last_forced: Option<u64>, now: u64, cooldown: u64) -> bool` |
| `hlc::observe_step`, `hlc::now_step` | `sync/src/hlc.rs` | `(state: (i64, i64), physical: i64, remote: i64, max_drift: i64) -> Result<(i64, i64), HlcError>` |
| `bucket::admit` | `ratelimit/src/memory.rs`, mirrored by the Lua script | `(tokens: u64, last: u64, now: u64, rate, capacity, cost) -> (bool, u64, u64)` |
| `FixedWindow::at` | `ratelimit/src/amount_budget.rs` | already pure |
| `is_public_address`, `permits` | `egress/src/policy.rs` | already pure |
| `check_expiry`, `decode_canonical`, `parse_expires` | `core/src/media_grant.rs` | already pure |
| `signature_timestamp_valid` | `suite/src/domain/mod.rs:730` | already pure |
| `is_own_native_scheme`, `validate_return_url` | `suite/src/domain/mod.rs` | already pure |
| `feature_subject` | `suite/src/platform/plan_http.rs:35` | already pure |
| `hub_url` | `suite/src/platform/hub.rs` (`url`, `encode_segment`) | `(base: &Url, segments: &[&str]) -> Result<Url, HubError>` |
| `lease_predicate_holds` | `jobs/src/store.rs` (spec mirror of the SQL predicate) | `(row: LeaseRow, worker: &str, attempt: u32, now: i64) -> bool` |
| `replay_outcome`, `fence_rejects` | `erasure/src/store.rs` (spec mirror of `REJECTED_SESSION_SQL`) | `(op: OperationRow, auth_time: Option<i64>, now: i64) -> bool` |
| `deletion_status_outcome` | `erasure/src/api.rs`, `keycloak.rs` | `(status: u16, body_says_missing: bool) -> Result<(), IdentityDeletionError>` |
| `export::split` | `data-subject/src` | already pure |

## 5. Implementation plan

Each phase ends with:

- the gate green (`sh scripts/ci-gate.sh`, Rust tests with `--include-ignored`);
- the threat model updated;
- one PR per finding or per tool.

Day estimates assume one engineer who knows the crate.

### Phase 0: tooling spike (3 days)

Goal: prove each tool runs against this workspace before anyone writes 40 harnesses.

1. **Kani.**
   - Install it (`cargo install --locked kani-verifier && cargo kani setup`).
   - Build one trivial harness each in `baukit-ratelimit`, `baukit-egress`, `baukit-sync`, `baukit-auth`,
     `baukit-jobs`, `baukit-erasure` and `baukit-suite`.
   - The risk is crates that pull in `aws-lc-sys`, `ring`, `sqlx` or `tokio`. Kani compiles the whole crate even
     though it only analyses code reachable from the harness.
   - If a crate fails, put the kernels in a `kernels` module behind a cargo feature that excludes the heavy deps,
     or in a tiny `baukit-kernels` crate with no dependencies.
2. **Lint config.** Add `cfg(kani)` so `-D warnings` doesn't trip on it:
   `[workspace.lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }`.
3. **Verus.** Verify `hlc::now_step` in a scratch crate, then choose a layout:
   - (a) The production kernel lives inside `verus!` and depends on `vstd`. This only works if `vstd` builds on
     MSRV 1.95 and passes `cargo deny`.
   - (b) A verified twin lives in `rust/verus/`. A Kani harness proves twin and production equal within bounds,
     and a proptest checks them over random inputs.

   I recommend (b) unless (a) turns out clean, because (a) puts a research toolchain in the build path of 12
   products.
4. **TLA+.** Add `tla2tools.jar` and a Java runtime to `deploy/tools.toml` with a pinned sha256, like the other
   validators. Model-check a 20-line toy spec in a new `models` gate step.
5. **fast-check.** Add it to the TS workspace catalog. Confirm vitest picks up `*.property.test.ts`, and that
   `fc.scheduler` works with the fake timers the packages already use.
6. **Migration linter.** Try squawk on `rust/crates/*/migrations`. Check what it flags today against F-58.
7. **Workflow audit.** Run the zizmor config at the repo root with `--persona auditor`, and see what it reports on
   `release.yml`.
8. **Pin and record.** Pin every tool version in `deploy/tools.toml` or `mise.toml`, and record the versions and
   spike results in this doc.

Exit: one green harness, model or check per tool in CI, and the layout decisions written down.

### Phase 1: fix the high findings, test first (15 to 17 days)

Order by exposure. Each item is one PR: a failing regression test, the fix, then the property or proof named in
the "catches it" column.

1. **Rate limiting end to end: F-36, F-01, F-02, F-37.** One PR series, because fixing F-36 alone exposes F-01.
   - Pass connect info in `spawn_listener`.
   - Rewrite `resolve_client_ip` around `select_forwarded_ip`, flip the default hops to 0, and add the IP
     pre-limit outside `establish_principal`.
   - Regression test: a real socket, where the bucket is exhausted from 127.0.0.1 and 127.0.0.2 still gets 200.
     This test is gate G-02.
   - Replace the test that asserts the old XFF fallback. Add K-01 and K-02.
   - Release notes and `docs/migrations/` entry, because hops=0 is a breaking default.
2. **JWKS amplification: F-03, plus F-12.**
   - `ArcSwap<JwkSet>`, a forced-refresh cooldown, a single-flight refresh, stale-on-error, and tolerant key
     parsing.
   - Regression test: 1,000 distinct unknown kids within the cooldown cause at most 1 fetch, and a valid token
     verifies concurrently with no added latency.
   - Add K-05 and P-03.
3. **Client identity signals: F-38 and F-41.** These interact, because a foreign 401 can trigger the wrong erase.
   - `handleProfileErased(subject)`, and session-generation stamps in api-runtime.
   - Origin and base-path checks before attaching a token or honouring a 401 signal.
   - Regression tests: the stale 401 after an account switch, and a cross-origin 401.
   - Write M-07 in this phase. Add fast-check properties C-01 and C-02.
4. **Erasure correctness: F-40 and F-42.**
   - Provider-specific 404 parsing and a startup probe.
   - A template helper that runs `guard_subject` inside the product's write transaction.
   - Regression tests: a misconfigured base URL never completes an operation, and a concurrent write after the
     fence check cannot survive erasure.
   - Write M-06 in this phase.
5. **Organization access: F-39 and F-47.**
   - Public eviction, wired to membership and seat events in the template.
   - The role returned from `is_member`, plus `require_role`.
   - Standalone mode requires a membership port.
   - An injectable clock.
   - Add P-15 and M-08.
6. **Job lease: F-05, with F-14.**
   - Add `attempt` to every terminal statement's predicate and to `JobLease`.
   - Regression test on Postgres: same `worker_id`, lease expires, reclaim, and the stale `complete` returns
     `LeaseLost`.
   - The runner survives one attempt's error, and `retry_after` is clamped.
   - Write M-02. Add P-10.
7. **HLC drift: F-04.**
   - `max_drift_ms` in Rust and TS. My suggested default is 24 h; products can tighten it.
   - A server-side check helper for push handlers.
   - Add K-09 and D-01.
8. **Sync pull: F-06 and F-07.**
   - Write M-01 first and let TLC produce counterexample traces. That settles whether the findings are real or only
     reachable by following the docs.
   - Then ship `PullWindow { hi, after: (revision, id) }` from `guard_pull_cursor`.
   - Fix the README and the `lib.rs` example.
9. **Release gating: F-46.** A day of work that closes a path to all 12 products.
   - A protected environment, an ancestry check, and a build/publish job split.
   - A workflow-shape check: a job with `id-token: write` must have an `environment:` and must not run installs.
10. **Security test helpers: F-60.** Strengthen `auth_negative` and `cross_tenant`, because every later product
    test relies on them. This is gate G-05.
11. **Threat model.** Rows for every high finding, plus the drift and new boundaries from 3.4.

### Phase 2: generated-product conformance gate (6 days)

F-36 shipped because every test checked a piece, and nothing checked the composed product. This phase tests what
`baukit new` actually generates, in every fixture flavor.

| ID | Check | How |
|---|---|---|
| G-01 | Composed router is testable | Move `compose_api` from `bin/api.rs` into `__app__-bin/src/lib.rs` as `pub async fn compose_api(config, deps) -> Router`. |
| G-02 | Real-socket limiter test | Serve through `serve_listener_pair`. Exhaust the bucket from 127.0.0.1; 127.0.0.2 still gets 200. |
| G-03 | Layer coverage on every route | Take the inventory from OpenAPI plus an explicit list of late routes (`/auth/session/*`, `/mcp`, `/.well-known/oauth-protected-resource*`, `/platform/inbound`), and fail when a late router is not listed. For every route, assert: the second request returns 429 at quota 1; an oversized body returns 413; `nosniff` and the CSP are present. |
| G-04 | Auth versus limiter order | A counting fake verifier. Bad bearers reach the verifier at most once per IP allowance, and random kids cause a bounded number of JWKS fetches. |
| G-05 | Security helpers against the composed router | `auth_negative` and `cross_tenant` run on G-01's router, with positive controls. Every unsecured operation must be declared public with a reason. |
| G-06 | Migration safety | squawk over every shipped migration. A test that applies all baukit migrations twice (re-runnable). An old-binary test: migrate, then run the previous release's store tests against the new schema. |
| G-07 | proptest over the composed router | Random method, path and `Authorization` (garbage, random kid, alg confusion). Assert no 2xx on secured paths, no 5xx, security headers on every response, and bounded JWKS fetches. |
| G-08 | Static rules in `baukit check` | `axum::serve(` without connect info is an error. In `*-bin` crates, every `.merge(x)` needs `x` wrapped in `rate_limited(` or `protect_routes(`. The chart's `opsService.type` must be `ClusterIP`. Origins are `https://` outside local and never `null`. Close the F-62 bypasses. |
| G-09 | Rendered-output snapshots | Snapshot the rendered `api.rs`, `rate_limits.rs` and auth wiring per flavor as content, not only file names, so a dropped layer shows up in review. |

### Phase 3: Kani harnesses (6 days)

Each harness lives in a `#[cfg(kani)] mod proofs` next to its kernel, with its bounds in the doc comment.

| ID | Kernel | Property |
|---|---|---|
| K-01 | `select_forwarded_ip` | Non-interference: for any two XFF lists that agree on the rightmost `hops` entries, the result is equal. Lists up to 6 entries, hops 0 to 4. This fails on today's code. |
| K-02 | `select_forwarded_ip` | The result is the peer or the entry at `len - hops`, never anything else. |
| K-03 | `is_public_address`, `permits` | Equals a spec table built from the IANA special-purpose registries, for every `u32`, and for `u128` with symbolic upper 32 bits over each listed prefix (mapped, NAT64, 6to4, Teredo, ULA, link-local, multicast). Adds `168.63.129.16`. |
| K-04 | `ipv4_mask`, `ipv6_mask` | Prefix 0 to 128: no panic, and the mask has exactly `prefix` leading ones. |
| K-05 | `jwks_refresh_allowed` | Over any sequence of 8 symbolic `now` values, at most `1 + span / cooldown` return true. |
| K-06 | `claims_time_valid` | No overflow for any `i64` pair. Ok iff `nbf - skew <= now <= exp + skew`, computed in `i128`. |
| K-07 | `ApiTokenFormat::is_well_formed` | Never panics on inputs up to 96 bytes. Accepts iff the prefix, length and alphabet match. |
| K-08 | `constant_time_eq` | Equals `==` for lengths up to 8. |
| K-09 | `hlc::observe_step`, `now_step` | For all valid states, physical times and remotes: Ok means the result is above the previous state and above the remote, with wall time at most `max(physical + max_drift, state.wall)`. Err means the state is unchanged. No panic. |
| K-10 | `hlc::encode`, `decode`, `compare` | Round trip over all valid `(wall, counter)`. Compare is a total order consistent with encode. |
| K-11 | `bucket::admit` | One step: tokens never exceed capacity and never underflow, and admission implies `cost <= tokens after refill`. |
| K-12 | `FixedWindow::at` | `reset_at - window <= now < reset_at`, no overflow. |
| K-13 | `media_grant::check_expiry` | No panic. Ok implies `now < expires <= now + max_ahead`. |
| K-14 | `media_grant::decode_canonical`, `parse_expires` | Accept implies re-encoding gives the same bytes. |
| K-15 | `signature_timestamp_valid` | Equals `-300 <= ts - now <= 300` in `i128`, for every `i64` pair. |
| K-16 | `is_own_native_scheme` | Accepts iff `s == own`, or `s == own + "-dev"` with the dev flag on. Strings up to 12 bytes. |
| K-17 | `lease_predicate_holds` | A stale attempt number never satisfies it. Paired with a Postgres test that the SQL and the mirror agree on a generated table of rows. |
| K-18 | OAuth `parse_token` | Accept iff length 43 and canonical base64url round trip. |
| K-19 | `ip_bucket` | Two IPv6 addresses share a bucket iff they share the configured prefix. IPv4-mapped addresses bucket as their IPv4. |
| K-20 | `feature_subject` | An `Organization(x)` result implies `x` is the token's organization. |
| K-21 | `hub_url` | The path always starts with `/platform/v1/`, and segments never inject `/`, `?`, `#` or dot segments. |
| K-22 | `fence_rejects`, `replay_outcome` | Matches the documented truth table for every combination of operation status, `auth_time` and `erased_at`. Paired with a Postgres agreement test against `REJECTED_SESSION_SQL`. |
| K-23 | `deletion_status_outcome` | Ok only for 2xx, or 404 with a provider "missing" body. Every other status is an error. |
| K-24 | `export::split` | Every part is within the limit, and concatenating the parts round-trips. |

### Phase 4: fuzzing with real oracles (6 days)

Existing targets keep running, with fixed oracles:

- **`jwt_verify`.** It cannot reach the claim logic today, because the fuzzer can't produce a validly signed
  token. Rewrite it as structure-aware:
  - `arbitrary` generates the header and claims (aud as string, array, empty or null; exp, nbf, azp, client_id,
    scope, nonce, typ).
  - A selector picks one of the 4 profiles or the ID-token path, and the harness signs with the mock key.
  - Oracle: the result equals an independent reference `expected(claims, profile, now)`, error kind included.
  - Swap the mock JWKS server for an in-process key source, so random kids stop costing HTTP calls.
  - Use a fixed clock, so the seed token never expires mid-campaign.
- **`oauth_state_parse`.** The cookie oracle `text.contains(name)` is always true. Replace it with an exact-name
  parse check, and add the canonical round trip.
- **`http_headers`.** Check that the input was canonical (`format(parse(v)) == v`), not only the reverse. Build
  the runtime once.
- **`mcp_jsonrpc`.** Fuzz Host, Origin and protocol headers, and tokens with varied scopes. Assert the id echo,
  202 for notifications, and scope denial.
- **`sync_hlc`.** Interleave `now`, `observe` and reopen with a varying physical clock.
- **`media_grant_verify`.** Fuzz the method and a two-key ring.

New targets:

| ID | Target | Input | Oracle |
|---|---|---|---|
| Z-01 | `xff_resolve` | arbitrary header list, peer, hops | K-01's non-interference, on full-size inputs |
| Z-02 | `jwks_parse` | arbitrary JWKS body | no panic; a token signed by K verifies iff K is published with a matching kid, kty and alg |
| Z-03 | `egress_destination` | arbitrary URL string | if accepted and the host is an IP literal, `permits(ip)` holds |
| Z-04 | `webhook_verify` | secret, ts, id, body, sig | accepts exactly when `sig` is the canonical encoding of `sign(...)` |
| Z-05 | `receipt_verify` | arbitrary envelope | accept implies PAE verifies under a known key and the payload re-validates |
| Z-06 | `suite_event_decode` | `PlatformEventDecoder::decode` | Ok implies owner equals userId and source equals config |
| Z-07 | `suite_ticket` | `ParsedTicket::parse`, `verify_ticket` | Ok implies the re-serialized claims satisfy `verify_claims` |
| Z-08 | `suite_return_url` | `validate_return_url`, `relay_callback` | the emitted `Location` re-parses to an allowed origin, scheme, host and path |
| Z-09 | `session_cookie` | raw Cookie headers into `CookieSettings::read`, `SessionCredential::parse` | exact-name match, round trip |
| Z-10 | `expo_responses` | `ExpoTicketResponse`, `ExpoReceiptResponse`, plus `send_chunk` ticket mapping | no panic; mismatched lengths are an error, never a misattributed ticket |
| Z-11 | `identity_deletion_payload` | `IdentityDeletionHandler` job payload | no panic; only well-formed subjects reach a deleter |
| Z-12 | `idp_delete_url` | arbitrary subjects into `ApiDeleter` and the Keycloak `url()` builder | the path never leaves `/users/{subject}` |

Corpus seeds come from `fixtures/`. The PR lane stays at 60 s per target. A new scheduled workflow runs 30 minutes
per target nightly, caches the corpus, and opens an issue with the artifact on a crash.

### Phase 5: property and state-machine tests (8 days)

| ID | Subject | Property |
|---|---|---|
| P-01 | `InMemoryRateLimitStore::check_at` | Over random time traces, admitted cost in any interval is at most `capacity + rate * length`, including under eviction pressure (F-15). |
| P-02 | amount budget | Consume and release across window boundaries never over-admit (F-16). |
| P-03 | JWKS cache with a fake key source and clock | Fetches per window are bounded for any kid stream. Valid kids verify during cooldown, and during an IdP outage within the stale grace. |
| P-04 | Lua token bucket vs `bucket::admit` | Same decisions on the same trace (Redis via testcontainers). |
| P-05 | audience, azp and profile truth table | `validate_token_profile` equals a reference table for every combination of aud shape, azp, client_id and profile. |
| P-06 | `ApiToken::is_active_at` | Equivalent to `verify_at` returning Ok, including the `expires_at == now` boundary. |
| P-07 | BFF `lifetimes` | `idle <= absolute` and `max_age <= idle_timeout`. |
| P-08 | `EntitlementCache::store` | The revision never decreases. Memory and Postgres agree. A refresh that says free makes `current()` free (F-08). |
| P-09 | `webhook_signing_input` | Injective over (ts, id, body). Any bit flip in ts, id, body or sig fails verification. |
| P-10 | Jobs, state machine on Postgres | Actions: claim, complete, fail, expire, cancel, stall, with several slots sharing one worker id. Invariant: a succeeded job was completed by its latest attempt. Code-level twin of M-02. |
| P-11 | Sync, state machine on Postgres | Actions: write (several rows per revision), tombstone, purge, paged pull at RC and RR, erase and recreate. Invariant: the replica equals the reference model after a full pull. Code-level twin of M-01. |
| P-12 | Vault | Dropping or substituting a field behaves as documented (F-30). Truncation below the 16-byte tag fails. |
| P-13 | Push registry, state machine on Postgres vs `MemoryDeviceRegistry` | Register, rotate, unregister, invalidate, erase owner, takeover attempt. Each token has at most one owner, each owner at most `cap` devices, invalidation never removes a post-send registration, and a takeover without the install secret fails (F-43). |
| P-14 | Erasure service, state machine on Postgres | Erase and preflight with random keys, subjects and `auth_time` values against a model of replay, conflict and fence semantics. A concurrent product write through the template helper never survives an erase (F-42). |
| P-15 | Organization plans with an injectable clock | After a removal event, `is_member` is false. Without an event, it's false once the TTL has passed. Billing role never passes `require_role(Member)` (F-39). |
| P-16 | Credits, state machine against a scripted hub | Each reservation goes Pending to Settled, Released or Expired exactly once. `sum(charged) <= sum(reserved)`. The same key with the same subject and amount returns the same id, a different amount conflicts, and a replay after settle is refused (F-48). |
| P-17 | Pending receipts under concurrent pollers | Every ticket is taken eventually and never by two pollers in the same window. |

### Phase 6: TLA+ protocol models (10 days)

Models live in `models/<name>/`, each a `.tla` spec plus a `.cfg` with small constants, so TLC finishes in under 5
minutes in CI. Each model gets a paired state-machine test (P-10, P-11, P-14, P-15, and the fast-check models in
Phase 8). That test drives the real implementation with the same actions and checks the same invariants. The
pairing is what connects a model to the code, so no model runs alone.

| ID | Model | Actors and actions | Invariants |
|---|---|---|---|
| M-01 | SyncPull | 2 writers, 1 puller, 2 tables, page size 1, rows per revision 1 or 2, isolation RC or RR, purge with horizon | NoLostUpdate (cursor `c` means every row version at or below `c` was applied), NoResurrection after purge, HorizonSafety. It should produce counterexamples for F-06 and F-07 on today's design, and none after the fix. |
| M-02 | JobLease | 2 slots with one worker id, a second worker, a stalling handler, lease expiry, cancel | At most one terminal write per lease. Succeeded implies completed by the latest attempt. Attempts never exceed max. Liveness: a pending job is eventually claimed under weak fairness. |
| M-03 | BffSessionFamily | exchange; refresh split into IdP rotate, then crash or cancel, then commit; replay; logout; purge; 2 tabs; IdP reuse detection | At most one live head per family. Reuse after grace implies the family is revoked. No access token for a revoked family. Liveness: an honest client that retries is never logged out (finds F-11). |
| M-04 | SuiteLink | start, callback, two concurrent completes, two exchanges, revoke, expiry | At most one link per request. Consumed implies a link id. Completion is idempotent. An active initiator link implies the authorizer link exists. |
| M-05 | SuiteInbox | hub redelivery, prepare outside the transaction, cancellation, receipt purge at 30 days | Exactly one effect for revision-guarded handlers. At-least-once delivery. The same id with a different body gives 409. |
| M-06 | ErasureSaga | Erase (lock, replay check, guard, product erase, operation row, fence, enqueue, commit, inline attempt), Worker (claim, delete, complete, lease expiry), Login, Request (fence check then write, lock-free and guarded variants), OperatorFence; a crash at any step; the IdP answering 404 nondeterministically | Completed implies no product rows and no IdP account. No admitted request with `auth_time <= erased_at`. Liveness: every operation ends completed or failed. It should find F-40 and F-42. |
| M-07 | ClientAccountSwitch | tabs, a broadcast session token, a lifecycle per tab, in-flight requests stamped with a token, an outbox per subject, server erase; actions SignIn, SignOut, Deliver, Recheck, Push (reads the token at send time), Respond(profile_erased), HandleProfileErased, EraseServer | Every push from partition p carries subject(p)'s token (F-53). Erase touches only erased subjects (F-38). No push for s after EraseServer(s) (F-54). |
| M-08 | MembershipCache | hub, N replicas each with member and grant caches, a clock, remove, suspend and role-change events, optional event delivery | `Removed(s,o) and now - t_remove > TTL` implies not granted. With event-driven eviction, removal implies not granted once the event is delivered (F-39). |

M-01, M-02, M-06 and M-07 are written during Phase 1, because they settle or confirm high findings. The rest come
here.

### Phase 7: Verus proofs (8 to 12 days)

Three kernels, chosen because Kani's bounds are exactly where their bugs would hide:

| ID | Kernel | Theorem | Proof |
|---|---|---|---|
| V-01 | HLC | For every sequence of `now` and `observe` calls of any length, outputs strictly increase, and wall time never exceeds the maximum physical time seen plus `max_drift`. | `rust/verus/src/hlc.rs` `replay`. The bound as written is false: 1000 `now` calls at a stalled clock roll the counter over and push the wall time 1 ms past it. Proven instead: `wall * 1000 + counter <= max(start, (max_physical + max_drift) * 1000 + 999) + calls`. |
| V-02 | Token bucket | For every admission trace of any length, admitted cost over `[t0, t]` is at most `capacity + rate * (t - t0)`. | `rust/verus/src/bucket.rs` `replay`, for one key with a fixed scaled quota, starting full at `t0`, `t` the latest request time. |
| V-03 | Pull cursor progress | With the Phase 1 tuple cursor, every page either advances the cursor strictly or returns `hasMore = false`, and no row with `(revision, id)` above the cursor and at or below `hi` is skipped. | `rust/verus/src/pull.rs` `page_progress_and_no_skip`. Needs `limit >= 1`; with `limit = 0` every row is skipped. |

The layout follows the Phase 0 decision. With the twin layout, each Verus kernel gets a Kani harness proving twin
and production equal within bounds, plus a proptest over random inputs. Without that link, the Verus proof would
describe code nobody runs.

### Phase 8: TypeScript and cross-language (8 days)

fast-check properties and models:

| ID | Package | Property |
|---|---|---|
| C-01 | api-runtime | `Authorization` is present iff the request origin equals the base origin and the path is under the base path. Arbitraries include `//`, `\`, `%2F`, absolute URLs and `Request` objects. `onProfileErased` and `onUnauthorized` never fire for off-origin responses. At most one 401 replay. POST and PATCH without an idempotency key are sent exactly once on 5xx (F-41). |
| C-02 | data-contracts lifecycle | `fc.commands` with `fc.scheduler` over `ScopedPersistenceLifecycle` and `InMemoryStorePool`. Commands: select(s), clear, eraseActive, profileErased(s), sessionExpired, and resolving or failing open, close and erase. Invariants: at most one open handle; `current()?.subject` is the last requested subject; erase only touches the signalled subject (F-38). Code-level twin of M-07. |
| C-03 | data-contracts registry | Two adapter instances over one shared store under the scheduler: every resolved subject stays registered and removals stick (F-56). `parseRegistry` on arbitrary JSON either throws the identity error or round-trips, and never writes on corrupt input. `canonicalIdentity` is injective, including `:` and astral characters. |
| C-04 | data-contracts web storage | For namespaces n1 and n2 that differ, operations on n1 never change what n2 sees (F-55). |
| C-05 | data-contracts import, export, erasure | `prepareImportEnvelope` returns a preview or `ImportEnvelopeError`, never another error type, with `rowCount <= maxRows`; a write failing at step k leaves state unchanged. `encodeCsv` round-trips through an RFC 4180 parser, and no neutralized cell starts with a trigger. `eraseProductProfile` signs out iff the server erase succeeded and never erases locally after a server failure. |
| C-06 | sync-client | `rankPushBatch` covers each changeId exactly once and never merges distinct entities (F-09). `validatePullPage`. The revisioned-writes queue keeps the last enqueued value on accept. HLC monotonicity with two instances on one `HlcStorage` (F-19). |
| C-07 | auth-web, auth-node | `fc.commands` model of `BrowserSessionClient` with fake fetch, locks, broadcast channel and clock skew: no token after logout, at most one refresh in flight, no refresh storm under skew (F-13). `completeCallback` over arbitrary URLs. `readEndSessionUrl` returns only http(s). `NodeTokenCache.parseDocument` has no prototype pollution and round-trips. Stale lock recovery (F-28). |
| C-08 | suite-client | `verifyTicket` rejects any mutated segment. `authorizeUrl` must be https on an allowed origin (F-49). `createSuiteNativeIntentValidator`, `authorizeEndpoint`. |
| C-09 | purchases-core | `PurchaseFlow` command model: a grant implies the latest server answer for the current subject granted it (F-57). |

Differential vectors. A Rust example per domain (`cargo run -p baukit-sync --example gen_vectors`) writes
`fixtures/<domain>/generated-v1.json` from a fixed seed. CI regenerates the file and diffs it, the same way
`openapi_drift` works today. TS and njs tests consume it.

| ID | Domain | Coverage |
|---|---|---|
| D-01 | HLC | random `now` and `observe` traces, including rollover, exhaustion and drift rejection |
| D-02 | events | each validation rule from F-20, timezone offsets, future bounds |
| D-03 | tickets | duplicate keys, `1.0` and `1e9`, non-canonical Ed25519 R and A, `manageUrl` edge cases (F-33) |
| D-04 | media grants | Rust signer and verifier vs njs `verifyMediaGrant` under njs and QuickJS; key ids with a trailing `\n`, 64-byte key ids, 10-digit expiry edges |
| D-05 | return URLs | uppercase scheme, fragment, backslash path, dot segments, trailing-dot host, `:443`, leading whitespace, scheme collisions; Rust `validate_return_url` vs TS `authorizeEndpoint` |
| D-06 | CSV export | native floats (`1e21`, `1e-7`, `-0`), full-width triggers, Rust `encode_csv` vs TS `encodeCsv` (F-63) |
| D-07 | limits | `trimmedUnicodeScalarCount`, `compactJsonUtf8Bytes` and import string measurement on lone surrogates vs Rust `limits.rs` |

### Phase 9: remaining medium and low findings (10 days)

Every F-xx not pulled into an earlier phase. Each gets a regression test, plus the property named for it above.
These change wire or storage behaviour and need a version note and template updates:

- F-11 (BFF refresh)
- F-18 (`x-suite-replay` signing)
- F-43 (push install secret)
- F-55 (web storage key format)
- F-58 (migration rewrites)

## 6. CI integration

| Lane | Trigger (via `scripts/ci-gate.toml`) | Budget | Runs |
|---|---|---|---|
| `kani` | changes under `rust/crates/**` | 15 min, harnesses of affected crates only | `cargo kani -p <crate>` |
| `kani-full` | nightly | 60 min | every harness |
| `verus` | changes to the kernels or `rust/verus/**` | 10 min | `verus` on each proof crate |
| `models` | changes to `models/**` or the code each model maps to (sync, jobs, browser session, suite link and inbox, erasure, data-contracts lifecycle, suite organizations) | 10 min | TLC on each `.cfg` |
| `conformance` | changes to `templates/**`, `cli/**`, `baukit-runtime`, `baukit-ratelimit`, `baukit-auth`, `baukit-test` | inside the existing generated-fixture gate | G-01 to G-07 per flavor |
| `migrations` | changes to any migration | 5 min | squawk, double apply, old-binary test (G-06) |
| `workflows` | changes to `.github/**` | 1 min | zizmor auditor persona, actionlint, release-shape check |
| `fuzz` | unchanged | 60 s per target | existing `scripts/fuzz.sh` |
| `fuzz-nightly` | schedule | 30 min per target | corpus cache, issue on crash |
| `vectors` | changes to `fixtures/**`, generators, TS consumers | 2 min | regenerate and diff |

Each lane gets a `step_*` in `scripts/ci-gate.sh` and a CI job that calls `scripts/ci-gate.sh --step NAME`, like
every other job. Tools install from `deploy/tools.toml` into `.tools/`.

Harness bounds go in a generated table (`scripts/list-proofs.py` reads the doc comments), appended to this doc, so
reviewers can see what each proof covers and what it doesn't.

## 7. What products get

The fixes reach every product on the next baukit bump. The behaviour changes get release notes and a
`docs/migrations/` entry:

- F-02 (XFF default)
- F-36 (connect info)
- F-37 (IP pre-limit)
- F-11 (BFF refresh)
- F-38 (subject-scoped erase signal)
- F-43 (push install secret)
- F-47 (standalone membership port)
- F-55 (web storage keys)

These also go into `templates/`:

1. The composed-router conformance tests from Phase 2, so each product keeps checking its own wiring after it
   diverges from the template.
2. The strengthened `baukit-test` helpers with positive controls, plus the declared-public list.
3. The `baukit-test` pull-conformance helper from Phase 1, run in each product's sync tests, because the pull SQL
   is product-owned and F-06 and F-07 live there.
4. The `guard_subject` write helper from Phase 1, so product writes are fenced inside their transaction.
5. A fast-check dependency and one example property in the web and mobile templates.
6. The `arbitrary`-based layout for the strict template's fuzz scaffold, from Phase 4.

## 8. Order and total

| Phase | Days | Depends on |
|---|---|---|
| 0 tooling spike | 3 | none |
| 1 high fixes, plus M-01, M-02, M-06, M-07 | 15 to 17 | 0 for Kani, TLC and fast-check |
| 2 generated-product conformance gate | 6 | 1.1 (G-02 is that PR's regression test) |
| 3 Kani | 6 | 0, kernels from 1 |
| 4 fuzzing | 6 | none, can run alongside 3 |
| 5 properties and state machines | 8 | 1 |
| 6 remaining models (M-03 to M-05, M-08) | 6 | 0 |
| 7 Verus | 8 to 12 | 0, 3 |
| 8 TS and differential | 8 | 1 |
| 9 medium and low | 10 | none |

Roughly 75 to 85 engineer-days in total. Phases 0 to 2 close every high finding and stop the F-36 class of bug from
coming back. If time runs short, protect those three phases.

