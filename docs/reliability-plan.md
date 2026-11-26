# Reliability plan

Owner: Claude (orchestrator). Implementers: subagents per the current orchestrator setup, with disjoint
directory ownership. The orchestrator re-runs every acceptance command, reads the diff and looks for new
suppressions or skipped checks before ticking a box.

Status markers: `[ ]` open, `[~]` in progress, `[x]` done and verified, `[-]` dropped (reason in the log).

Scope, in this order:

1. The Rust crates every product links (`baukit-egress`, `-http`, `-runtime`, `-ops`, `-jobs`, `-auth`,
   `-suite`, `-push`, `-erasure`, `-data-subject`, `-ratelimit`, `-telemetry`) plus one new crate,
   `baukit-resilience`.
2. The TypeScript clients products ship (`api-runtime`, `sync-client`, `auth-web`, `auth-native`,
   `pwa-web`).
3. The templates that wire both into generated products (`templates/backend`, `templates/worker`,
   `templates/web`, `templates/mobile`).
4. The platform: `deploy/chart/baukit-app`, `deploy/platform/*` bases, `deploy/observability*`.
5. A baukit release, then product adoption.

Evidence was collected on 2026-11-26 against baukit `c2c0aec2` (v0.30.6) by reading code. Nothing was
reproduced under load yet; Phase FX turns the important findings into failing tests before they are
fixed. Paths are relative to the baukit root. The platform hardening plan
(`docs/platform-hardening-plan.md`) already covers security, policy and most of the platform's own
reliability; this plan only lists what is still open after it.

## What we found, in short

The serving path is in better shape than I expected. Panics become opaque 500s, every request has a
timeout and a body limit, liveness stays static while readiness checks the database, the rate limiter
fails open, and the egress client bounds DNS, connect, request and body. That is a solid floor.

The weak spots are the places where one slow or dead dependency turns into a fleet-wide problem:

- **Nothing stops calling a dependency that is down.** There is no circuit breaker in the egress client.
  The one breaker that exists (suite links) opens after at least 200 failed HTTP calls, only when the
  owner enqueues again, and never closes on its own.
- **Identity is a single point of failure with a lock in front of it.** The OIDC verifier holds one
  mutex across the JWKS fetch and never serves a stale key set. When the IdP is down past the 300 s TTL,
  every authenticated request fails, one after the other, each waiting up to 5 s for the lock.
- **Overload and outages look like bugs to clients.** Excess requests queue for 30 s and then get a
  504. A database outage returns a 500. Neither carries `Retry-After`, so clients cannot back off.
- **Workers are fragile.** One transient store error ends the runner and the process. Every rollout
  strands in-flight jobs for 15 minutes behind their leases. Retries have no jitter, so they come back
  in lockstep after an outage. The queue-age alert probably fires every day because of how the gauge is
  computed.
- **Nobody can see outbound health.** `baukit-egress` emits no metrics, so there is no alert for "the
  payment provider is failing" and nothing a breaker could report into.
- **Production runs one API pod.** The GitOps base pins `replicas: 1`, so every node drain or kured
  reboot is an outage, and the PDB never renders.

The circuit breaker is the right headline, but it pays off only together with three other changes:
`Retry-After` on every 503, jittered backoff everywhere, and a job outcome that defers work without
burning an attempt. Without those, an open breaker just converts a slow failure into a fast failure that
the job runner then retries ten times in lockstep.

## Principles

1. **Fail fast, say when to come back.** A request that cannot succeed returns quickly with a status that
   tells the caller it is transient (`503`, `429`) and a `Retry-After`. Callers honour it. This holds for
   every hop: browser to API, API to provider, worker to peer.
2. **Every retry is bounded and jittered.** No unbounded loops, no lockstep. Full jitter is the default
   in Rust and TypeScript, and both sides share one set of test vectors.
3. **Dependency state is per process and observable.** Breakers, caches and limiters live in the
   process, not in Redis. Each one exports its state as a metric with bounded labels.
4. **Stale beats down for read-mostly data.** JWKS, discovery documents, entitlements and ticket keys
   keep serving the last good value for a bounded window when the source fails.
5. **Startup tolerates a blip; runtime tolerates an outage.** A process waits with backoff for its
   dependencies inside the startup probe window instead of crash-looping. Once running, it degrades
   (readiness off, 503s) instead of exiting.
6. **Every failure mode ships with a test that triggers it.** A breaker gets a test where the
   destination dies and calls stop. A drain change gets a test with a slow request in flight. A platform
   change gets an e2e case that kills or drains something.
7. **Domain logic stays framework-free.** The breaker, backoff and deadline logic is pure, takes a clock
   port, and sits in its own crate. HTTP clients, job runners and the TS client are adapters around it.
8. No product is live, so breaking API changes land directly, without transition switches.

## Decisions

These are defaults the plan assumes. Each one can be overruled before its phase starts.

| ID | Decision | Default | Why |
|---|---|---|---|
| D1 | Where the breaker lives | A new framework-free crate `baukit-resilience` holds the state machine, jittered backoff and a deadline type behind a `Clock` port. `baukit-egress` applies it inside `GuardedClient`. | The jobs runner needs the same backoff, the auth caches need the same cooldown, and the TS client mirrors the same state machine. Keeping it pure makes the cross-language vectors cheap to test. It is too small to be a framework. |
| D2 | Breaker key | One breaker per `(scheme, host, port)` inside a client, shared across clones through an `Arc`. Metrics label by a logical `destination` name set on the client, never by host. | The suite peer client shares one `GuardedClient` across all peers, so a per-client breaker would let one dead peer block the others. Hosts are unbounded label values; configured names are not. |
| D3 | Breaker algorithm | Count-based sliding window of the last 20 calls. Opens at 50% failures with at least 10 calls in the window, or at 5 consecutive timeouts. Open for 30 s, doubling on each re-open up to 5 min, with 20% jitter. Half-open admits one probe; a success closes, a failure re-opens. | A rate window ignores a single bad call on a busy destination but still opens fast on a quiet one through the consecutive-timeout rule. The numbers are starting points and live in `BreakerPolicy`. |
| D4 | What counts as a failure | `RetryClass::Unavailable`, `Timeout` and 5xx without `Retry-After`. `429` or `503` with `Retry-After` opens the breaker for exactly that delay without counting as a failure. `Permanent` and `Revoked` never count. | A 400 means our request is wrong, not that the destination is down. A provider that says "come back in 60 s" should not get a call before then. |
| D5 | Default on | The breaker is on by default in `EgressOptions` with the D3 policy. `without_circuit_breaker()` exists for tests and one-shot tools. | Products build their own provider clients on `GuardedClient`. Opt-in would leave most of them unprotected. |
| D6 | Shared breaker state across replicas | No. State stays per process. | Each pod finds out within a few calls. Redis-shared state adds a dependency to the failure path it is meant to protect. |
| D7 | Open breaker inside a job | A new `JobError::deferred(delay)` outcome releases the job back to `pending` without consuming an attempt. Deferrals are counted in a `deferrals` column and capped by `WorkerConfig.max_deferrals` (default 100); past the cap a deferral consumes an attempt like `retryable_after`. | Today `retryable_after` burns an attempt, so a 30 s open window would exhaust a 10-attempt suite delivery in minutes. The cap keeps a permanently dead destination from deferring forever. |
| D8 | Edge circuit breaker in Traefik | Not enabled. Traefik gets explicit timeouts and retries on connection errors only. | With one or two API pods, an edge breaker turns a partial failure into a full outage. The app-level load shedding (IN1) and 503s cover the same ground with better signals. |
| D9 | Browser breaker | A per-origin cooldown in `api-runtime`, not a full state machine: after a `503`/`429` with `Retry-After`, or 5 consecutive network failures, requests to that origin fail fast until the cooldown ends. | Browsers mostly need "stop hammering, tell the UI". The full half-open machinery adds little for one user. |
| D10 | Fault injection tool | Testcontainers stop/pause for unit-level dependency loss, plus `ghcr.io/shopify/toxiproxy` (newest stable, pinned by digest) for latency and partial failure in `baukit-test`. Chaos Mesh is out of scope. | Toxiproxy runs in one container, needs no cluster, and covers latency, resets and bandwidth. Chaos Mesh on a one-node k3d is heavy for what it adds. |

## Findings catalog

Only findings verified by reading the code are listed. RL = reliability finding. Fix IDs point to the
tasks below.

### Outbound calls

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| RL-01 | No circuit breaker, bulkhead or per-destination concurrency limit on any outbound call. | `rust/crates/baukit-egress/src/client.rs` (no breaker or semaphore) | RC1, CB1 |
| RL-02 | `baukit-egress` emits no metrics. The only per-destination signal is `{prefix}_suite_deliveries_total`. No alert can see a failing provider. | `baukit-egress/src/client.rs:300-309` (tracing span only), `baukit-suite/src/services/runtime.rs:118-121` | CB2 |
| RL-03 | The OIDC verifier holds one `tokio::Mutex` across the JWKS fetch, so every verification, including cached kids, waits behind a fetch of up to 5 s. After the TTL a failed fetch is returned and the old set is never used. No cooldown after failure. | `baukit-auth/src/verifier.rs:682-704` | ID1 |
| RL-04 | OIDC discovery and browser-session discovery run once at startup with no retry. An IdP blip during a rollout crash-loops every new pod. | `templates/backend/__auth__/.../identity.rs:15-18,97`, `baukit-auth/src/browser_session/router.rs:87-94` | ID2 |
| RL-05 | `HubClient` holds the token-cache lock across the client-credentials POST, has no failure cooldown, and replays any request once after a `401`, idempotent or not. | `baukit-suite/src/platform/hub.rs:262-263,315-325,343-375` | ID3 |
| RL-06 | Entitlements never serve an expired snapshot. With the hub down, users whose cache lapsed get `503`, although `grace_until` exists. | `baukit-suite/src/platform/cache.rs:52-53,184-227` | ID4 |
| RL-07 | `TicketVerifier` has a separate refresh lock and a 5 s cooldown, but no stale fallback. | `baukit-suite/src/platform/tickets.rs:14,382-422` | ID1 |
| RL-08 | One timeout per client and no per-request deadline on `EgressRequest`. An inbound request with 2 s left still waits the full 10 s outbound timeout. | `baukit-egress/src/client.rs:46-90`, `options.rs:8-11` | IN8 |
| RL-09 | `ExpoPushSender::send` returns on the first failing chunk and drops tickets from chunks already accepted. A retry resends those pushes and their receipts are never polled. | `baukit-push/src/expo.rs:294-295` | CB6 |
| RL-10 | The suite link breaker is hardcoded at 20 consecutive terminal failures in four places, is evaluated only on the owner's next enqueue or revoke, ignores retry outcomes, has no half-open state and is reset only by the user-facing re-enable route. | `baukit-suite/src/domain/mod.rs:31,941-962`, `migrations/003_suite_lock_order.sql:40-43`, `adapters/postgres/outbox.rs:207,226,274`, `adapters/http.rs:460` | CB4 |

### Inbound serving and process lifecycle

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| RL-20 | Overload queues. `ConcurrencyLimitLayer` (1024) has no load shedding in front, so excess requests wait unbounded for a permit and fail as `504` after 30 s, without `Retry-After`. | `baukit-http/src/middleware.rs:175-180,298-312`, `options.rs:9`, `error.rs:216-221` | IN1 |
| RL-21 | A database outage maps to `500`. `baukit-http` has no `503` constructor. Clients and proxies cannot tell an outage from a bug. | `templates/backend/backend/crates/__app__-api/src/lib.rs:433-435,444-447`, `baukit-http/src/error.rs` | IN2 |
| RL-22 | Rate limiting in closed mode returns `429` with `Retry-After` set to the whole quota period when the store is down. | `baukit-ratelimit/src/axum_layer.rs:128,217` | IN2 |
| RL-23 | One global request timeout, no per-route override. axum `serve` runs hyper without a timer, so there is no header-read timeout, no idle timeout and no connection cap (slowloris exposure). | `baukit-http/src/options.rs:104`, `baukit-runtime/src/lib.rs:753-762` | IN3 |
| RL-24 | The API drain budget (grace 30 s minus preStop 5 s = 25 s) is shorter than the request timeout (30 s). Listener drain, the erasure task and telemetry flush share that one deadline, so the flush can get 0 s. Worker drain equals the full grace, so it ends at SIGKILL. | `deploy/chart/baukit-app/templates/_helpers.tpl:145,148,244-252`, `values.yaml:91-94`, `templates/backend/.../bin/api.rs:325-347` | IN4, PL2 |
| RL-25 | The generated API uses `serve_listener_pair` (`ShutdownOrder::Together`), so `/readyz` stops with the API and never reports draining. Only the preStop sleep keeps traffic off a terminating pod. | `templates/backend/.../bin/api.rs:11,325`, `baukit-runtime/src/lib.rs:655-668` | IN4 |
| RL-26 | Startup exits on the first failed connect to Postgres, Redis or the IdP. A short blip puts every new pod into CrashLoopBackOff, whose backoff outlasts the blip. | `templates/backend/.../bin/api.rs:86-92,188`, `templates/worker/.../bin/worker.rs:48-53`, `bin/migrate.rs:27-29` | IN5 |
| RL-27 | No `statement_timeout`, `lock_timeout` or `idle_in_transaction_session_timeout` anywhere. A dropped request's query keeps running and holds a pool connection. | grep over `rust/`, `templates/`, `deploy/` | IN6 |
| RL-28 | The template queries `&self.pool` directly and never uses `baukit_ops::acquire`/`begin`, so `db_pool_acquire_duration_seconds` and `db_pool_acquire_timeouts_total` stay at zero in every product, and their alerts never fire. | `templates/backend/.../__app__-postgres/src/lib.rs:68-226`, `baukit-ops/src/postgres.rs` | IN6 |
| RL-29 | The HTTP histogram's top bucket is 10 s against a 30 s timeout, so the slow tail is invisible. No metric shows concurrency-limit saturation or shed requests. No process or Tokio runtime metrics. | `baukit-telemetry/src/lib.rs:108-110`, `docs/platform/telemetry-spec.md` §2.1 | IN7 |

### Workers and delivery

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| RL-40 | One store error or one `LeaseLost` ends `WorkerRunner::run`. The template wraps it in `RestartPolicy::FailProcess`, so a Postgres failover blip restarts the worker and the dropped `JoinSet` aborts every in-flight attempt. | `baukit-jobs/src/runner.rs:198-262`, `templates/worker/.../bin/worker.rs:139-151` | WK1 |
| RL-41 | Leases are fixed at claim (15 min default, 900 s in generated values) with no heartbeat and no release on shutdown. Every rollout strands in-flight jobs for up to 15 min, and a job on its final attempt fails for good. | `baukit-jobs/src/store.rs:502,515`, `runner.rs:128-138,216-218,261-263`, `templates/backend/deploy/values.yaml:8-10` | WK2 |
| RL-42 | `retry_delay` is plain exponential with no jitter. After an outage every failed job retries at the same moment. | `baukit-jobs/src/runner.rs:423-429` | RC1, WK3 |
| RL-43 | `worker_queue_oldest_age_seconds` is `MIN(created_at) WHERE status='pending'` over the whole table. It counts future `run_after` rows (the daily retention slot), ignores the runner's job types, and the API's erasure runner reports the same value under another label. `BaukitWorkerQueueOldestAgeHigh` likely fires daily, and its text uses `$labels.job_kind`, which the gauge lacks. | `baukit-jobs/src/store.rs:649`, `templates/worker/.../__app__-worker/src/lib.rs:168-175`, `deploy/observability/alerts/baukit.rules.yml:169-177` | WK4 |
| RL-44 | Missing worker metrics: queue depth, failed-row count, lease expiries. Cancellations count as `failure`, inflating `BaukitWorkerFailureRateHigh`. | `baukit-jobs/src/metrics.rs`, `runner.rs:349-355` | WK4 |
| RL-45 | Claims are global FIFO. A burst of deliveries to one hanging peer fills all slots (5 by default) and starves other destinations and job kinds. | `baukit-jobs/src/store.rs:523` | WK5 |
| RL-46 | A recurring job whose slot fails terminally stops the chain, because `enqueue_initial` returns the existing failed row for that slot. Retention stops until a restart lands in a later slot. | `baukit-jobs/src/recurring.rs:56`, `templates/worker/.../__app__-worker/src/lib.rs:58-66` | WK6 |
| RL-47 | Data-subject and erasure operations stay `pending` forever when their job exhausts attempts. The inline erasure path holds `SELECT ... FOR UPDATE` across the IdP HTTP call. | `baukit-data-subject/src/server/worker.rs:403`, `baukit-erasure/src/store.rs:533-551` | WK7 |
| RL-48 | Replaying failed jobs means hand-written `UPDATE` statements from the runbook. | `docs/runbooks/app/worker-failure-rate-high.md` | WK8 |
| RL-49 | Each idle replica issues about 25 queries/s: an age query and a claim transaction every 250 ms, plus a cancellation poll per running job every 250 ms. | `baukit-jobs/src/runner.rs:203-207,405-417`, `store.rs:508-529` | WK9 |

### Clients

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| RL-60 | `api-runtime` sets no request timeout. A backend that accepts and hangs leaves queries pending. | `typescript/packages/api-runtime/src/fetch.ts:47-80`, `templates/web/web/src/api.ts:10-13` | CL1 |
| RL-61 | `api-runtime` retries 502/503/504 only, ignores `Retry-After` and never retries `429`. | `api-runtime/src/retry.ts:25` | CL1 |
| RL-62 | The generated web app uses a bare `new QueryClient()`, whose defaults stack 3 more retries without jitter on top of `api-runtime`'s, plus refetch on window focus. | `templates/web/web/src/main.tsx:13` | CL2 |
| RL-63 | The sync scheduler never retries without a product `delayMs`, ignores `SyncRateLimitError.retryAt`, and fires `online`/visibility triggers and the periodic interval with no jitter. | `typescript/packages/sync-client/src/scheduler.ts:28-37,95,107-121,208-220` | CL3 |
| RL-64 | The browser-session refresh POST has no timeout and runs while holding the cross-tab Web Lock. A hanging backend blocks every tab. A retryable refresh failure throws even while the current token is still valid. | `typescript/packages/auth-web/src/browser-session-client.ts:120-134,224-229,251,381-390` | CL4 |
| RL-65 | Mobile token renewal fires once. On failure it records an error and never reschedules. | `templates/mobile/__auth__/mobile/src/auth.ts:209-227` | CL4 |
| RL-66 | `pwa-web` `networkFirst` has no network timeout, so a hanging API never falls back to cache. No template has a React error boundary or an "unavailable" banner. | `typescript/packages/pwa-web/src/fetch-handler.ts:49-66`, `templates/web/web/src/main.tsx:15-21` | CL5 |
| RL-67 | `compose.yaml` has no `restart:` policies and the backend service has no healthcheck. | `templates/backend/compose.yaml:61,91` | CL6 |

### Platform

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| RL-80 | Production runs one API and one worker pod with autoscaling off. The PDB renders only above one replica, so every drain and kured reboot is an outage. The HPA template has no `behavior` block. | `templates/backend/deploy/gitops/base/helmrelease.yaml:48,72,107`, `deploy/chart/baukit-app/templates/pdb.yaml:5`, `templates/hpa.yaml` | PL1 |
| RL-81 | The readiness check timeout (2 s) equals the probe `timeoutSeconds` (2), so a slow database turns into probe timeouts. One shared single-instance CNPG restart flips every product unready at once. | `baukit-ops/src/lib.rs:103`, `deploy/chart/baukit-app/values.yaml:120` | PL2 |
| RL-82 | The web workload has no PDB, no preStop and no startupProbe. | `deploy/chart/baukit-app/templates/pdb.yaml:1`, `web.yaml:82-89` | PL2 |
| RL-83 | Flagger cannot gate a chart-deployed app. Its metric templates filter on `rollout="canary"`, which nothing emits. The chart selector lacks the `baukit.dev/workload` label Flagger selects on. No template generates a Canary, and e2e never tests promotion or rollback. | `deploy/platform/flagger/metric-templates.yaml:19,28,53`, `deploy/chart/baukit-app/templates/_helpers.tpl:59-62` | PL6 |
| RL-84 | The SLO pack covers availability only, from the server side. No latency SLO, no ticket tier (1d/3d), no error-budget panel. When every pod is down, `or on() vector(0)` reads as zero burn. | `deploy/observability/recording-rules/baukit-red.rules.yml:44-94`, `alerts/baukit.rules.yml:6-46` | PL7 |
| RL-85 | No CNPG `Pooler`. `max_connections` is unset (CNPG default 100) while each pod pools 10. Two-instance overlays use async replication, so failover can lose acknowledged commits. | `deploy/platform/postgres-cluster/cluster.yaml:8`, `overlays/baremetal/components/postgres/kustomization.yaml:19` | PL4 |
| RL-86 | Platform Redis has one alert (ACL load). Nothing covers master down, Sentinel quorum, replication link or evictions. | `deploy/observability-platform/alerts/redis.rules.yml:4` | PL5 |
| RL-87 | Traefik sets no entrypoint `respondingTimeouts`, no `serversTransport` forwarding timeouts and no retry middleware. Baremetal keeps one replica although the README says multi-node runs two. | `deploy/platform/traefik/helm-release.yaml:24-200`, `overlays/baremetal/components/traefik/kustomization.yaml`, `traefik/README.md:11` | PL3 |
| RL-88 | Fault tests cover redis-ha master kill and backup/restore only. Two tests the hardening plan marks done do not exist: R1's "rollout under a curl loop with no failed requests" and R2's `kubectl drain --dry-run=server`. | `deploy/platform/e2e/tests/`, `docs/platform-hardening-plan.md:518-525` | FX3 |
| RL-89 | Generated products have no dependency-loss test. `baukit-test` already has `stop_master` for Redis, but no template uses it. | `templates/**/tests`, `rust/crates/baukit-test/src/redis.rs:108` | FX1, FX2 |
| RL-90 | No alerts for outbound error rate, MCP request errors, push or outbox delivery, or DB acquire latency (the histogram exists). | `deploy/observability/alerts/baukit.rules.yml` | CB2, PL7 |

## Target architecture

```text
rust/crates/baukit-resilience/       new, framework-free, no tokio in the public API
  src/backoff.rs                     full jitter + decorrelated jitter, seeded RNG port
  src/breaker.rs                     BreakerPolicy, CircuitBreaker, Outcome, Admission
  src/breaker_set.rs                 keyed breakers with bounded size (LRU on idle keys)
  src/deadline.rs                    Deadline, remaining(), min_with(timeout)
  src/clock.rs                       Clock port + SystemClock + ManualClock (test feature)
  src/stale.rs                       StaleCache<T>: fresh TTL, stale window, failure cooldown, single flight
fixtures/resilience/
  breaker-v1.json                    event sequences -> expected states, shared by Rust and TS
  backoff-v1.json                    seeded jitter vectors
rust/crates/baukit-egress/
  src/breaker.rs                     adapter: RetryClass -> Outcome, CircuitOpen error
  src/metrics.rs                     egress_* metrics
rust/crates/baukit-jobs/
  store: release(), heartbeat(), requeue_failed(), deferrals column
typescript/packages/api-runtime/
  src/cooldown.ts                    per-origin cooldown (D9), consumes fixtures/resilience
docs/platform/resilience-contract.md new: the end-to-end contract (503 + Retry-After, jitter, breaker)
```

`baukit-resilience` depends on nothing but `std` and `thiserror`. `StaleCache` takes an async fetch
closure, so the crate stays runtime-free while the adapters run it under tokio.

## Metrics to add to the telemetry spec

All names go into `docs/platform/telemetry-spec.md` §2 and `SPEC_METRICS` /
`HISTOGRAM_METRICS` in `deploy/observability/lint/check-metric-names.py` in the same change that emits
them.

| Metric | Type | Labels | Owner |
|---|---|---|---|
| `egress_requests_total` | counter | `destination`, `status`, `outcome` (`success`, `unavailable`, `timeout`, `rate_limited`, `revoked`, `permanent`, `circuit_open`, `saturated`) | `baukit-egress` |
| `egress_request_duration_seconds` | histogram | `destination` | `baukit-egress` |
| `egress_circuit_state` | gauge | `destination` (0 closed, 1 half-open, 2 open; worst key per destination) | `baukit-egress` |
| `egress_circuit_transitions_total` | counter | `destination`, `to` | `baukit-egress` |
| `http_requests_shed_total` | counter | `reason` (`concurrency`, `draining`) | `baukit-http` |
| `worker_queue_depth` | gauge | `queue`, `status` (`pending`, `running`, `failed`) | `baukit-jobs` |
| `worker_lease_expired_total` | counter | `queue` | `baukit-jobs` |
| `worker_job_deferrals_total` | counter | `job_kind` | `baukit-jobs` |
| `auth_key_cache_requests_total` | counter | `source` (`oidc`, `ticket`), `result` (`fresh`, `stale`, `fetched`, `failed`) | `baukit-auth`, `baukit-suite` |

`worker_job_runs_total` gains `outcome="cancelled"`. The HTTP histogram gains a `30` bucket.
`destination` values are static strings set in code or config (`suite-peer`, `hub`, `oidc`, `expo`,
`keycloak-admin`, product provider names), never hosts.

## Phase RC: resilience core

- [x] **RC1 `baukit-resilience` crate** (D1, RL-01, RL-42): `Backoff` with full jitter (default) and
  decorrelated jitter, `CircuitBreaker` with the D3 policy, `BreakerSet<K>` capped at 256 keys with
  idle eviction, `Deadline`, the `Clock` port and a `ManualClock` behind a `test-support` feature.
  Every state transition is a pure function of (state, outcome, now).
  Acceptance: unit tests per transition; `fixtures/resilience/breaker-v1.json` and `backoff-v1.json`
  replayed by a Rust test; a proptest that the breaker never admits more than one half-open probe and
  that open duration never exceeds the cap. `cargo deny`, MSRV 1.95, version coherence (the crate joins
  the release train).
- [x] **RC2 `StaleCache<T>`** (principle 4): fresh TTL, stale-on-error window, failure cooldown, single
  flight that does not block readers of the cached value, and a forced refresh for unknown keys that
  respects the cooldown. Acceptance: tests with `ManualClock` for fresh hit, stale serve while the
  source fails, cooldown stops a fetch per request, expiry of the stale window, and N concurrent
  readers triggering one fetch.
- [ ] **RC3 Resilience contract doc**: `docs/platform/resilience-contract.md` states the end-to-end
  rules (who returns 503, who honours `Retry-After`, default policies, which destinations exist) and
  links `integration-reliability.md`, which drops its own retry prose in favour of it.

## Phase CB: circuit breaker and outbound metrics

- [ ] **CB1 Breaker in `GuardedClient`** (D2-D5, RL-01): `EgressOptions::with_destination(name)`
  becomes required (constructor argument), `with_circuit_breaker(policy)` and
  `without_circuit_breaker()`, `with_max_in_flight(n)` per key (default 32). The breaker check runs
  before DNS resolution, so an open circuit costs no lookup. An open circuit returns
  `EgressError::CircuitOpen` whose `retry_class()` is `RetryAfter(remaining)`; a full bulkhead returns
  `EgressError::Saturated` with `RetryAfter(1 s)`. Neither message contains the URL.
  Acceptance: scripted server tests (`baukit-test` HTTP fixture) where the destination returns 503s
  and the call count stops at the threshold; recovery through one probe; a 429 with `Retry-After: 2`
  holds the circuit for 2 s without counting a failure; a 400 never opens it; two destinations on one
  client stay independent; clones share state.
- [ ] **CB2 Egress metrics** (RL-02, RL-90): the four `egress_*` metrics from the table, spec and lint
  updated, plus recording rules and alerts: `BaukitEgressErrorRatioHigh` (per destination, traffic
  floor), `BaukitEgressCircuitOpen` (open for more than 5 min), `BaukitEgressLatencyHigh`. Each alert
  gets a promtool test that fires and one that stays quiet, a runbook under `docs/runbooks/app/`, and a
  panel in `baukit-service-overview.json`.
  Acceptance: `--step metric-names`, `--step observability-rules`, `--step observability-tests`.
- [~] **CB3 Deferred job outcome** (D7): `JobError::deferred(delay)`; store method
  `release(job_id, worker_id, run_after, consume_attempt: bool)` that resets to `pending`, decrements
  `attempts` when not consuming, and increments `deferrals`. Migration adds `deferrals integer not null
  default 0`. `WorkerConfig.max_deferrals` (default 100). A helper maps `EgressError` with
  `CircuitOpen`/`Saturated` to `deferred` and everything else to the existing classes.
  Acceptance: store conformance tests (memory and Postgres, `--include-ignored`) for deferral without
  attempt use, the cap, and `worker_job_deferrals_total`.
- [ ] **CB4 Suite delivery on the breaker** (RL-10): the peer client sets `destination("suite-peer")`
  and `deliver_claimed` maps an open circuit to `deferred`. The per-link counter stays as the per-user
  signal but moves out of SQL into the domain constant only (one definition, the CHECK constraint
  references nothing), is evaluated when the delivery finishes instead of on the next enqueue, and
  gains automatic half-open: a disabled link gets one probe delivery after 1 h, doubling to 24 h.
  Acceptance: suite tests where a dead peer receives at most the breaker threshold of calls across
  three links, links to a healthy peer keep delivering, and a disabled link re-enables itself after the
  peer recovers.
- [ ] **CB5 Hub and ticket keys on the breaker**: `HubClient` (`destination("hub")`), `TicketVerifier`
  key source and entitlement fetches go through the breaker. Covered together with ID3 and ID4.
- [ ] **CB6 Push** (RL-09): Expo sender uses `destination("expo")`; `send` returns per-chunk outcomes
  (`accepted` tickets plus the failing chunk's error) instead of discarding accepted tickets.
  Acceptance: a scripted Expo server failing the second of three chunks; tickets from chunk one are
  persisted and polled, a retry resends only chunks two and three.
- [ ] **CB7 Remaining adopters**: browser-session IdP (`idp`), erasure Keycloak/API (`keycloak-admin`),
  MCP introspection (`mcp-introspection`), OIDC JWKS (`oidc`, with ID1). Each gets one test that an
  open circuit fails fast with the mapped error code.
- [ ] **CB8 Template wiring**: every generated `GuardedClient` passes a destination name;
  `templates/backend/docs` shows how a product provider client picks its name and maps
  `CircuitOpen` in a job handler. Snapshot bless, `FIXTURE_FLAVORS="backend auth platform"`.

## Phase ID: identity and read-mostly caches

- [ ] **ID1 JWKS on `StaleCache`** (RL-03, RL-07): the OIDC verifier and `TicketVerifier` move to
  `StaleCache` with a 300 s fresh TTL, a 1 h stale window for known kids (never for unknown kids), a
  10 s failure cooldown, and no lock held across the fetch for cached reads.
  `auth_key_cache_requests_total` exported.
  Acceptance: a test where the JWKS endpoint dies after the first fetch, the TTL passes, and 100
  concurrent verifications of a known kid succeed with one fetch attempt and no request waiting on the
  fetch; an unknown kid during the outage fails fast with `UnknownKeyId`; after the stale window
  verifications fail with a stable error code.
- [ ] **ID2 Discovery without crash loops** (RL-04): `OidcVerifier::discover` and the browser-session
  router retry with `Backoff` inside a configurable startup budget (default 60 s, below the chart's
  150 s startup probe). A static `jwks_uri` in config skips discovery entirely, and the template
  documents it as the production default.
  Acceptance: a test with an IdP that comes up after 3 s; startup succeeds. One where it never comes
  up; startup fails with a clear error after the budget.
- [ ] **ID3 Hub client** (RL-05): token fetch moves to `StaleCache` (single flight, cooldown, no lock
  across the POST). The `401` replay is allowed only for requests built through an idempotent
  constructor (`HubRequest::idempotent`) or carrying an `Idempotency-Key`; the type makes the other
  case impossible to replay.
- [ ] **ID4 Entitlement grace** (RL-06): while the hub fails, serve the last snapshot until
  `grace_until`, mark the response `stale` for callers that care, and only then return `503` with a
  `Retry-After` taken from the breaker.

## Phase IN: inbound serving and startup

- [x] **IN1 Load shedding** (RL-20): a bounded wait in front of the concurrency limit
  (`HttpOptions::with_queue_timeout`, default 1 s). A request that cannot get a permit in time gets
  `503` with `Retry-After: 1` and the standard problem envelope, counted in `http_requests_shed_total`.
  The in-flight gauge stops counting queued requests.
  Acceptance: a test with `max_concurrency(2)` and five slow requests; three return 503 within the
  queue timeout, two succeed.
- [~] **IN2 Honest 5xx** (RL-21, RL-22): `ApiError::unavailable(retry_after)` returning `503` with
  `Retry-After`. The template maps `RepositoryError::Unavailable`, `LimitError::PolicyUnavailable` and
  pool acquire timeouts to it. Closed-mode rate limiting returns `503` with a short `Retry-After`
  instead of `429` with the quota period.
  Acceptance: template test with Postgres stopped (FX1) gets `503` plus `Retry-After`; ratelimit test
  for closed mode.
- [x] **IN3 Connection-level timeouts** (RL-23): `baukit-runtime` serves through hyper-util with
  `TokioTimer`, `header_read_timeout` (10 s), an idle keep-alive timeout and a connection cap.
  `baukit_http::route_timeout(duration)` adds a per-route override inside the global one.
  Acceptance: a slowloris test (open a connection, send half a header) closes after the header
  timeout; a route with a 1 s override times out at 1 s.
- [~] **IN4 Drain that finishes** (RL-24, RL-25): the template switches to
  `serve_listener_pair_with_shutdown_order(.., ShutdownOrder::OpsOutlivesApi)` so `/readyz` reports
  draining. `baukit-runtime` validates at startup that `request_timeout + flush_reserve <
  drain_timeout` and refuses to start otherwise. Telemetry flush gets its own 3 s reserve that the
  listener drain cannot consume. The chart computes drain as grace minus preStop minus 5 s, and the API
  grace default becomes 45 s.
  Acceptance: runtime test with a request that takes 20 s in flight at SIGTERM: it completes, readiness
  flips to 503 first, and the flush runs. A config with a 30 s request timeout and a 25 s drain fails
  validation.
- [ ] **IN5 Startup waits for dependencies** (RL-26, principle 5): a `baukit_ops::connect_with_backoff`
  for the Postgres pool and the Redis store, bounded by the same startup budget as ID2. The API starts
  with the rate-limit store degraded (fail-open path) when Redis is still down after the budget,
  instead of exiting. The migrate binary gets the same connect retry.
  Acceptance: template tests where Postgres starts 5 s after the API; readiness turns green without a
  restart.
- [~] **IN6 Database guardrails** (RL-27, RL-28): `DatabaseConfig` gains `statement_timeout` (default
  just under the request timeout), `lock_timeout` (5 s), `idle_in_transaction_session_timeout` (60 s),
  `max_lifetime` (30 min) and `idle_timeout` (10 min), applied in `after_connect`. The template
  repository uses `baukit_ops::acquire`/`begin` everywhere so the pool metrics fire. The worker pool
  gets its own `statement_timeout` from the job timeout.
  Acceptance: a Postgres test where `pg_sleep(60)` fails at the statement timeout and the connection
  returns to the pool; a template test that `db_pool_acquire_duration_seconds_count` increases.
- [x] **IN7 Saturation metrics** (RL-29): a `30` bucket on the HTTP histogram, process metrics
  (`process_*` from the exporter), and Tokio runtime metrics behind a feature where the pinned tokio
  allows it without `tokio_unstable`. Spec and lint updated.
- [ ] **IN8 Deadline propagation** (RL-08): the lifecycle middleware stores a `Deadline` in a task
  local; `GuardedClient` uses `min(own timeout, remaining)` and fails fast with `Timeout` when less than
  50 ms remain. Outbound requests do not send the deadline to third parties.
  Acceptance: a handler with 2 s left calling a destination that sleeps 5 s returns within about 2 s.

## Phase WK: workers and delivery

- [x] **WK1 Runner survives transient errors** (RL-40): store errors in the loop are retried with
  `Backoff` (capped at 30 s) and logged, readiness turns red while the store fails, and the loop keeps
  running. `LeaseLost` becomes a per-job warning plus `worker_lease_expired_total`. Only
  configuration errors and panics in the runner itself end `run`.
  Acceptance: a Postgres test that pauses the container for 10 s mid-run; the runner resumes, no
  in-flight attempt is aborted by the runner, readiness flips red and back.
- [x] **WK2 Heartbeat and release** (RL-41): `heartbeat(job_id, worker_id, lease)` extends
  `locked_until` every lease/3 while the handler runs. On drain the runner stops claiming, signals
  `JobCancellation`, waits up to the drain budget, and calls `release(.., consume_attempt: false)` for
  every attempt still running. The default lease drops to 60 s now that it is renewed; the generated
  values drop the 900 s override.
  Acceptance: a test that SIGTERMs a worker during a 2 min job; another worker claims it within a few
  seconds, the attempt count is unchanged, a final-attempt job is not failed.
- [ ] **WK3 Jitter** (RL-42): `retry_delay` uses `baukit_resilience::Backoff` with full jitter.
  Acceptance: the shared backoff vectors; a statistical test that 1000 jobs failed at once spread
  over the window.
- [~] **WK4 Honest queue metrics** (RL-43, RL-44): the age query uses `MIN(run_after)` over
  `status = 'pending' AND run_after <= now AND job_type = ANY($types)`. `worker_queue_depth` and
  `cancelled` outcome added. The alert text uses `queue`. Sample age and depth every 15 s, not every
  loop.
  Acceptance: a test where a future recurring slot exists and the gauge reports 0; promtool test for
  the corrected alert.
- [ ] **WK5 Per-key fairness** (RL-45): `NewJob::concurrency_key(key)` plus
  `WorkerConfig.max_per_key` (default 1 for suite deliveries, unset otherwise). The claim query skips
  pending rows whose key already has `max_per_key` running. Suite deliveries use the peer app as key.
  Acceptance: a hanging peer occupies at most `max_per_key` slots while deliveries to another peer
  complete. EXPLAIN check on the claim query with the new index, recorded in the PR.
- [ ] **WK6 Recurring chains do not break** (RL-46): on terminal failure of a recurring slot the runner
  schedules the next slot. `enqueue_initial` skips past a failed current slot.
  Acceptance: retention test where one slot fails all attempts and the next slot still runs.
- [ ] **WK7 Terminal reconciliation** (RL-47): data-subject and erasure operations move to `failed`
  with a stable code when their job fails terminally, and an alert fires on any failed operation. The
  inline erasure path commits its lock before the IdP call and re-checks state after it.
  Acceptance: tests for both terminal paths; a test that no transaction is open during the IdP call
  (pool size 1 and a concurrent query succeeds).
- [ ] **WK8 Requeue API** (RL-48): `PostgresJobStore::requeue_failed(job_type, ids)` and a
  `baukit-ops` admin hook the template exposes on the ops listener only. The runbook replaces its SQL
  with it.
- [ ] **WK9 Polling cost** (RL-49): idle backoff on empty claims (250 ms doubling to 5 s, reset on a
  claim), cancellation polled every 2 s, and an optional `LISTEN job_enqueued` wake-up.
  Acceptance: an idle runner issues fewer than 1 query/s after 30 s; enqueue latency with LISTEN stays
  under 500 ms.

## Phase CL: TypeScript clients and generated frontends

- [~] **CL1 `api-runtime` timeouts, Retry-After and cooldown** (RL-60, RL-61, D9): `requestTimeoutMs`
  (default 30 s) combined with the caller's signal; retry `429` and `503` when `Retry-After` is present,
  clamped to `maxRetryAfterMs`; the per-origin cooldown from D9 that fails fast with
  `ApiUnavailableError { retryAt }`. Backoff and cooldown replay `fixtures/resilience/*.json`.
  Acceptance: vitest for each path; a fake-timer test that 50 concurrent calls after a 503 with
  `Retry-After: 5` produce one request, not 50.
- [ ] **CL2 One retry layer in the web template** (RL-62): `QueryClient` with `retry: false`,
  `refetchOnWindowFocus` jittered or off, and a shared `isApiUnavailable` state from the cooldown.
- [x] **CL3 Sync scheduler defaults** (RL-63): default `delayMs` uses full jitter and
  `SyncRateLimitError.retryAt`; recovery triggers wait a random 0-5 s; the interval gets 10% jitter.
- [~] **CL4 Token refresh under outage** (RL-64, RL-65): the refresh POST gets a 10 s timeout; a
  retryable failure keeps using the current token until `expiresAt`; mobile renewal reschedules with
  jittered backoff on retryable failure and stops only on `invalid_grant`.
- [~] **CL5 Degraded UI** (RL-66): `networkFirst` gains a network timeout (default 4 s) before falling
  back to cache; the web and mobile templates get a top-level error boundary and an "unavailable, retrying
  at HH:MM" banner driven by the cooldown, with a11y tests and localized strings.
- [ ] **CL6 compose** (RL-67): `restart: unless-stopped` everywhere and a `/readyz` healthcheck for the
  backend service.

## Phase PL: platform

- [ ] **PL1 Two pods in production** (RL-80): the production overlay sets `api.replicas: 2` and
  `worker.replicas: 2` (WK2 makes two workers safe), so the PDB renders. The HPA template gains a
  `behavior` block (scale-down stabilization 300 s, scale-up at most 100% per 60 s) and
  `minReplicas: 2`. Local and testing stay at 1 to fit one node.
- [ ] **PL2 Probe and drain budgets** (RL-24, RL-81, RL-82): readiness `timeoutSeconds: 4` against a
  2 s check; drain computed as in IN4; worker grace defaults to drain budget plus 15 s; web gets a PDB,
  a preStop sleep and a startupProbe. `values.schema.json` rejects `drain >= grace`.
  Acceptance: chart unit tests in `scripts/test_chart_*`; the IN4 e2e case below.
- [ ] **PL3 Traefik timeouts and retries** (RL-87, D8): entrypoint `respondingTimeouts`
  (`readTimeout` 60 s, `writeTimeout` 60 s, `idleTimeout` 180 s), a default `ServersTransport` with
  `forwardingTimeouts` (`dialTimeout` 5 s, `responseHeaderTimeout` just above the app request
  timeout), and a chart-level retry middleware (`attempts: 2`, short initial interval) for connection
  errors. Baremetal runs two replicas as the README says. Check every field name against the pinned
  Traefik version before writing it.
- [~] **PL4 Postgres pooling and durability** (RL-85): a CNPG `Pooler` (PgBouncer, transaction mode)
  in the shared base, `max_connections` set explicitly, per-tenant `CONNECTION LIMIT` from the tenant
  component, and quorum synchronous replication on multi-instance overlays with the durability mode
  that keeps writes going when the replica is down (verify the field at the pinned CNPG version).
  The app's pool size and the pooler's `default_pool_size` are documented together in
  `local-footprint.md`.
- [ ] **PL5 Redis platform alerts** (RL-86): master down, Sentinel quorum lost, replication link down,
  evictions, memory near maxmemory, each with promtool tests and a runbook.
- [ ] **PL6 Flagger that can gate** (RL-83): add `baukit.dev/workload` to the chart selector (breaking,
  fine per principle 8), relabel `rollout` in the ServiceMonitor from the pod's canary/primary label,
  and add an opt-in Canary plus TraefikService to the template with error-ratio and p95 gates from the
  existing metric templates.
  Acceptance: an e2e case that rolls out a canary image returning 500s and asserts rollback, and one
  that promotes a healthy canary.
- [~] **PL7 SLOs beyond availability** (RL-84, RL-90): latency SLI recording rules (share of requests
  under the route's target, from the histogram), a 1d/3d ticket tier for both SLIs, an edge SLI from
  `traefik_service_requests_total` that sees "all pods down", an error-budget panel, and alerts for MCP
  errors, outbox/push delivery failures and DB acquire p95. Every alert gets promtool tests and a
  runbook.

## Phase FX: fault injection

FX tests come first in each phase where they apply: write the failing test, then the fix.

- [ ] **FX1 `baukit-test` fault fixtures** (D10, RL-89): `PostgresFault::{stop, pause, resume}`,
  `RedisFault` (reusing `stop_master`), and a `Toxiproxy` fixture with latency, reset-peer and timeout
  toxics. The image is pinned by version and digest per the pin policy; testcontainers labels follow the
  existing cleanup convention.
- [ ] **FX2 Generated dependency-loss tests** (RL-89): the backend template ships a test that stops
  Postgres and asserts `/healthz` 200, `/readyz` 503, API `503` with `Retry-After` within the queue
  timeout, and recovery after restart without a process restart. The worker template ships a test that
  pauses Postgres mid-job (WK1). Both run in every fixture flavor that has the component.
- [ ] **FX3 Platform e2e cases** (RL-88): the two missing hardening tests (rollout under a curl loop with
  zero failed requests; `kubectl drain --dry-run=server` with no blocking PDB), plus API pod kill under
  load with two replicas (zero 5xx at the edge beyond the shed budget), CNPG switchover with the app
  reconnecting, and a Flagger rollback (PL6). Each case runs inside `make platform-e2e` under the host
  locks and heavy-run slots.
- [ ] **FX4 Load smoke**: a short `k6` (pinned, digest) script in `deploy/smoke` that drives the
  generated API past its concurrency limit and asserts shed responses are 503 with `Retry-After`, p99
  of admitted requests stays under the target, and the process stays up.

## Phase REL: release and adoption

- [ ] **REL1 Release** with the new crate in the train: `scripts/check-version-coherence.py`, crates.io
  publish order updated for `baukit-resilience` (before `baukit-egress` and `baukit-jobs`), npm for the
  TS packages, tag.
- [ ] **REL2 Product adoption**: every product bumps, names its provider destinations (CB8), maps
  `CircuitOpen` to `deferred` in its provider job handlers, and runs its gate. Products that built their
  own retry loops around `GuardedClient` delete them.
- [ ] **REL3 Correct the hardening plan**: R1 and R2 back to `[~]` until FX3 lands, R5 ticked or reopened
  with the real remaining gaps (off-node default, PITR test, dedicated-DB backup checks).

## Verification per change

| Change touches | Run |
|---|---|
| `baukit-resilience`, `baukit-egress`, `baukit-jobs`, any crate | `scripts/ci-gate.sh --step` for the Rust lanes, `cargo test --manifest-path rust/Cargo.toml -- --include-ignored` (under the heavy-run slots), `cargo +1.95 check`, `cargo deny` when deps change |
| new metric names | `--step metric-names`, `--step observability-rules`, `--step observability-tests` |
| `templates/**`, public APIs the templates use | snapshot bless and diff, `make generated-fixture-gate` (all flavors before release, `FIXTURE_FLAVORS` subset while iterating) |
| `typescript/packages/**` | `corepack pnpm --dir typescript` build, format:check, lint, test; `make ts-browser-test` for `pwa-web` |
| `deploy/chart/**`, `deploy/platform/**` | the platform steps from the hardening plan, plus `make platform-e2e` for FX3 and PL cases |
| a version | `scripts/check-version-coherence.py` |
| before the release | full `scripts/ci-gate.sh` under the locks, `make platform-e2e`, `gh run list --workflow ci.yml` green on the pushed commit |

## Suggested execution order and parallelism

1. RC1 and RC2 (one agent, owns `rust/crates/baukit-resilience`, `fixtures/resilience`). FX1 in parallel
   (one agent, owns `rust/crates/baukit-test`).
2. In parallel, with disjoint ownership:
   - CB1, CB2 (owns `baukit-egress`, telemetry spec, metric lint, egress alerts)
   - CB3, WK1-WK4, WK9 (owns `baukit-jobs`)
   - ID1, ID2 (owns `baukit-auth`)
   - IN1, IN3, IN7, IN8 (owns `baukit-http`, `baukit-runtime`, `baukit-telemetry`)
   - CL1, CL3, CL4 (owns `typescript/packages`)
3. CB4, CB5, ID3, ID4, WK5 (owns `baukit-suite`); CB6, CB7 (push, erasure, mcp); WK6-WK8;
   IN2, IN4-IN6 (owns `baukit-ops`, `baukit-config`, `baukit-ratelimit`).
4. Templates in one agent (CB8, IN2/IN4-IN6 wiring, CL2, CL5, CL6, FX2), because snapshots serialize.
   PL1-PL7 in a second agent (owns `deploy/`). RC3 alongside.
5. FX3, FX4 under the locks, then REL1, REL2, REL3.

Directory ownership stays disjoint inside each step. The breaker and the deferred outcome (CB1, CB3) are
the critical path; everything in step 3 that adopts the breaker waits for them.

## Orchestration tracker

Integration branch `task/reliability` in `~/projects/baukit-worktrees/rel-int`. Each agent works in
`baukit-worktrees/rel-<agent>` on branch `rel/<agent>` from the integration head, and the orchestrator
merges after re-running the acceptance commands and reading the diff. Briefs and reports live in
`~/.cache/baukit-orchestrator/reliability/`. Implementers are Opus 5.5 at low effort, at most 8 at once.
Agents run fmt, clippy and non-Docker tests at `nice -n 19` with `CARGO_BUILD_JOBS=2`; only `rel-jobs`
and `rel-faults` may run targeted Docker tests. Full gates, fixture gates, browser tests and platform e2e
run in the orchestrator under the heavy-slot protocol (owner `baukit-rel`, one slot at most).

### Wave 1

| Agent | Tasks | Owns | Status |
|---|---|---|---|
| rel-core | RC1, RC2 | `rust/crates/baukit-resilience`, `fixtures/resilience`, workspace `Cargo.toml` entry | merged, verified (28 tests; review round fixed late outcomes deciding half-open) |
| rel-faults | FX1 | `rust/crates/baukit-test` | queued (host load) |
| rel-http | IN1, IN2 (`baukit-http` part), IN3, IN4 (`baukit-runtime` part), IN7 | `baukit-http`, `baukit-runtime`, `baukit-telemetry` | merged, verified (fmt, clippy, tests; shed log lowered to debug) |
| rel-jobs | WK1, WK2, WK4 (crate part), CB3 | `baukit-jobs` | merged, verified (two review rounds; Docker: jobs 51, suite 178, erasure 34, data-subject 53) |
| rel-ts | CL1 (without fixture replay), CL3, CL4 (`auth-web` part), CL5 (`pwa-web` part) | `typescript/packages/{api-runtime,sync-client,auth-web,pwa-web}` | merged, verified (4 packages build, format, lint, test) |
| rel-db | IN6 (crate part), IN2 (`baukit-ratelimit` part) | `baukit-config`, `baukit-ops`, `baukit-ratelimit` | merged, verified (unit tests plus Docker: 2 Postgres, 12 Redis) |
| rel-deploy | PL1, PL2, PL3, PL5 | `deploy/chart`, `deploy/platform/traefik`, `deploy/observability-platform` (redis alerts), `templates/backend/deploy` | running |
| rel-obs | PL4, PL7, the RL-43 alert text fix | `deploy/platform/postgres-cluster`, `deploy/observability` | merged, verified (platform-validate, observability-rules, observability-tests, metric-names, python-lint) |

Shared files (`docs/platform/telemetry-spec.md`, `deploy/observability/lint/check-metric-names.py`,
`rust/Cargo.toml`) get additive edits only; the orchestrator resolves merge conflicts.

### Wave 2 (after RC1 and RC2 merge)

| Agent | Tasks | Status |
|---|---|---|
| rel-egress | CB1, CB2, IN8 | running |
| rel-auth | ID1 (OIDC part), ID2, IN2 (browser session) | review round 1 |


CB1, CB2 (`baukit-egress`); WK3, WK5, WK6, WK8, WK9 (`baukit-jobs`); ID1, ID2 (`baukit-auth`); IN5, IN8;
CL1 fixture replay; CB4, CB5, ID3, ID4 (`baukit-suite`); CB6, CB7 (push, erasure, mcp); WK7; RC3.

### Wave 3

Templates in one agent (CB8, IN2/IN4-IN6 wiring, CL2, CL4 mobile part, CL5 template part, CL6, FX2);
PL6; FX3, FX4; REL1-REL3.

## Open questions

- D3 numbers are guesses until FX4 runs against a real product. Revisit after the first load smoke.
- Whether `LISTEN/NOTIFY` (WK9) survives the PgBouncer transaction mode in PL4. If not, the worker keeps a
  direct connection for the listener only.
- Whether Tokio runtime metrics (IN7) need `tokio_unstable` at the pinned tokio. If they do, drop them
  rather than add a cfg flag to every product.

## Log

### 2026-11-26

Evaluation of `c2c0aec2` by reading the crates, templates, TS packages and `deploy/`. Spot-checked
RL-03 (`verifier.rs:682-704`), RL-21 (`__app__-api/src/lib.rs:433-435`), RL-25 (`api.rs:325`,
`baukit-runtime/src/lib.rs:668`), RL-40 (`runner.rs:198-262`), RL-42 (`runner.rs:423-429`), RL-43
(`store.rs:649` plus the future `run_after` in the retention job), RL-62 (`main.tsx:13`), RL-80
(`helmrelease.yaml:48,72,107`), RL-83 (`metric-templates.yaml:19,28,53`) and RL-88 (no rollout or drain
case under `deploy/platform/e2e/tests`). Nothing implemented yet.
- rel-ts merged: CL1 (timeout, Retry-After, per-origin cooldown, `ApiUnavailableError`), CL3, CL4 web,
  CL5 web. Orchestrator re-ran all four packages green. Follow-ups for wave 3: the cooldown is per runtime
  instance, so CL2 must share one runtime in the template; a `Retry-After` cooldown is clamped to
  `maxRetryAfterMs` (30 s); the request timeout ends when headers arrive, not after the body.
- rel-http merged: IN1 (semaphore gate with 1 s queue timeout, 503 plus `Retry-After: 1`,
  `http_requests_shed_total{reason="concurrency"}` only), IN2 http, IN3 (`baukit_runtime::serve` on
  hyper-util, `ServerOptions`, `route_timeout`), IN4 runtime (`DrainBudget`, `ShutdownToken::with_budget`,
  `run_flush`), IN7 (30 s bucket, `/proc` process gauges, stable Tokio gauges). Wave 3 wiring: api.rs
  lines 294, 325, 343-347 and `examples/minimal-api` need `with_budget`, `OpsOutlivesApi` and
  `run_flush`. baukit-config has no fields for the queue timeout or `ServerOptions` yet. hyper arms the
  header timeout on idle keep-alive connections, so PL3 sets Traefik `idleConnTimeout` below 10 s.
- rel-core merged after one review round. The breaker now ties every outcome to a `Permit` with a
  generation ticket, so a late result from a call admitted before the circuit opened cannot close or
  re-open it. `CacheRead.refresh_failed` reports a failed refresh that served a stale value.
- rel-obs merged: CNPG `Pooler` (transaction mode, `max_prepared_statements` 200), `max_connections`
  150, quorum sync with `dataDurability: preferred` on baremetal, latency SLI and ticket-tier burn
  alerts, edge SLI on Traefik service metrics, MCP and DB acquire alerts, RL-43 alert text. Left open:
  tenant roles in chainsaw/keycloak/bootstrap have no connection limit; outbox and push alerts wait for
  metrics; products still connect to `-rw` (wave 3); the edge SLI needs Traefik `allowEmptyServices`
  (sent to rel-deploy); trivy may flag the pgbouncer image.
- rel-db merged: `DatabaseConfig` session timeouts and lifetimes, `baukit_ops::connect_pool`, and the
  ratelimit 503 in closed mode. Browser-session closed mode still answers 429 (needs a `baukit-auth`
  outcome, wave 2 auth agent). Decision for wave 3: `connect_options` sends the timeouts as startup
  parameters, which PgBouncer in transaction mode does not pass through. When a product moves to the
  pooler, the pooler enforces `query_timeout` and `idle_transaction_timeout`, and the tenant role gets
  `ALTER ROLE ... SET lock_timeout, statement_timeout`. `DatabaseConfig` then needs a switch that stops
  sending startup parameters.
- rel-jobs merged after two review rounds:
  - The first round made shutdown releases stop counting as deferrals (`ReleaseReason`), bounded the
    in-flight claim at shutdown, and updated the `job_outbox` 0005 test fixtures in suite, erasure and
    data-subject. `baukit-test` now accepts `cancelled`.
  - The Docker run found two real bugs, which the second round fixed. A paused Postgres hung loop
    queries: `WorkerConfig::store_call_timeout` (5 s) now bounds them. A job timeout during drain gave
    the attempt back instead of failing it.
  - Queue age now counts only the runner's own job types, as WK4 intends. A job type no runner handles
    therefore never ages any gauge. Worth an alert on `worker_queue_depth` for unknown types later.
  - Wave 3 template work: copy migration 0005 into the worker and auth templates, drop the 900 s lease,
    make readiness check `store_health()`, and review `RestartPolicy::FailProcess`.
  - Still open: the CB3 `EgressError` to `deferred` helper, which needs CB1.
