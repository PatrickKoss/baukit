# Migration guides

Each guide covers one breaking or behaviour change and says what a product has to do. Read the ones for the
crates and packages your product uses.

## Upgrading from 0.31.x

Do the steps in this order. The `baukit-jobs` changes are expand-only, so a 0.31.x binary keeps running
against them. The suite probe migration is not: stop the 0.31.x suite delivery processes before you
apply it.

1. Add the new SQL migrations to your product and apply them before deploying the new binary:
   - `baukit-jobs` `0005_baukit_jobs_deferrals.sql`, `0006_baukit_jobs_concurrency_key.sql`,
     `0007_baukit_jobs_validate_checks.sql`, then `0008_baukit_jobs_running_key_index.sql`, which
     runs outside a transaction. Expand-only. The auth and worker templates ship them as `0012` to
     `0015`, after `0011_identity_erasure_operation_id_guard.sql`. See
     [job-deferrals-and-concurrency-keys.md](job-deferrals-and-concurrency-keys.md).
   - `baukit-erasure` `0003_erasure_operation_id_guard.sql` after `0002`, shipped in the auth template as
     `0011_identity_erasure_operation_id_guard.sql`. It only replaces a trigger function. See
     [erasure-subject-rule.md](erasure-subject-rule.md).
   - `baukit-suite` `006_suite_delivery_probes.sql`, then `007_suite_validate_link_checks.sql`, after
     jobs `0008`. Not expand-only. See
     [fail-closed-503-and-grace.md](fail-closed-503-and-grace.md).
   - On shared platform Postgres, set the session timeouts and connection limit on each owner role. See
     [startup-budget-and-postgres-timeouts.md](startup-budget-and-postgres-timeouts.md).
2. Change the code that the new Rust APIs break:
   - [egress-circuit-breaker.md](egress-circuit-breaker.md): `GuardedClient::new` takes a destination name,
     `EgressError` gains `CircuitOpen` and `Saturated`, and job handlers map them with `egress_job_error`.
     Delete product retry loops around `GuardedClient`.
   - [job-deferrals-and-concurrency-keys.md](job-deferrals-and-concurrency-keys.md): `JobStore` gains
     `heartbeat`, `release` and `queue_depth`, `oldest_pending_age` takes job types, and
     `RunnerError::LeaseLost` is gone.
   - [http-overload-and-drain.md](http-overload-and-drain.md): build the shutdown token from a
     `DrainBudget` and flush telemetry with `run_flush`.
   - [startup-budget-and-postgres-timeouts.md](startup-budget-and-postgres-timeouts.md): `DatabaseConfig`
     gains five timeouts, pools come from `baukit_ops::connect_with_backoff`, and
     `RedisRateLimitStore::connect_if_enabled` takes a startup budget.
   - [erasure-subject-rule.md](erasure-subject-rule.md): erasure rejects subjects with control characters
     at the call with `ErasureError::InvalidSubject`, and the worker fails operations itself.
   - [fail-closed-503-and-grace.md](fail-closed-503-and-grace.md): new variants on `RateLimitOutcome`,
     `IdentityDeletionError` and `RedisTestError`, `EntitlementError` from `Entitlements`,
     `PushSendError` from `PushSender::send`, and `HubClient::post_json` is gone.
3. Check the configuration that changed defaults:
   - [http-overload-and-drain.md](http-overload-and-drain.md): overload is shed with `503` after a 1 s
     queue wait, `shutdown.drain_timeout` defaults to 35 s, and the `baukit-app` chart refuses grace
     periods that leave no drain. Delete the API and worker Deployments once before the chart upgrade.
   - [job-deferrals-and-concurrency-keys.md](job-deferrals-and-concurrency-keys.md): leases last 60 s and
     renew, and `worker.lease_duration_seconds` is gone from the worker template.
   - [startup-budget-and-postgres-timeouts.md](startup-budget-and-postgres-timeouts.md): a 60 s
     `startup.budget`, and `statement_timeout` must stay below `http.request_timeout`.
   - [fail-closed-503-and-grace.md](fail-closed-503-and-grace.md): a closed-mode rate-limit store error
     answers `503` instead of `429`, the JWKS stale grace is 1 h, and entitlements serve a stale snapshot
     until `grace_until`.
4. Run the new tests. The backend template adds `tests/postgres_outage.rs`, the worker template
   `tests/requeue_mount.rs`, and `baukit-test` adds fault fixtures (`PostgresTestContainer::fault`,
   `RedisTestContainer::fault`, `start_toxiproxy`) for your own outage tests. Strict products copy the
   gate result cache (`scripts/gate_results.py` and its test) and add `[quality.cache] never` to
   `baukit.toml`, as `templates/common/CHANGELOG.md` describes. `baukit doctor` requires both files.

## Upgrading from 0.30.x

Do the steps in this order. The database changes are expand-only, so the 0.30.x binary keeps running against the
new schema.

1. Add the new SQL migrations to your product and apply them before deploying the new binary:
   - `baukit-auth` `0003_baukit_auth_browser_session_record_ids.sql`. The auth template ships it as
     `0010_baukit_auth_browser_session_record_ids.sql`. See [boundary-hardening.md](boundary-hardening.md).
   - `baukit-suite` `005_suite_record_ids.sql`. See [boundary-hardening.md](boundary-hardening.md).
   - `baukit-push` `0005`, `0006`, then `0007`, which validates the `0005` check in its own transaction. See
     [push-install-secrets.md](push-install-secrets.md).
2. Change the code that the new Rust and TypeScript APIs break:
   - [job-lease-attempt.md](job-lease-attempt.md): `complete`, `record_failure` and `cancel` take a `&JobLease`.
   - [sync-pull-window.md](sync-pull-window.md): pull with a window, a `(revision, id)` cursor and a
     non-zero limit.
   - [suite-correctness-hardening.md](suite-correctness-hardening.md): the standalone plan takes a membership
     port, and v2 return URL signing has a deploy order.
   - [client-identity-signals.md](client-identity-signals.md): subject-scoped sign-out and erasure signals,
     delimited web storage keys.
   - [rate-limit-client-ip-and-buckets.md](rate-limit-client-ip-and-buckets.md): client IP selection behind
     proxies, IPv6 buckets, `BudgetWindow::index` as `i64`.
3. Check the configuration that changed defaults:
   - [auth-verifier-hardening.md](auth-verifier-hardening.md): access tokens with `nonce` are rejected, and the
     JWKS refetch cooldown delays a new signing key by up to 30 seconds.
   - [hlc-drift-bound.md](hlc-drift-bound.md): remote timestamps more than 24 hours ahead are rejected.
   - [data-subject-client-credentials.md](data-subject-client-credentials.md): the data-subject API accepts
     only client-credentials tokens.
4. Run the stricter test suites. [security-suites-and-composed-api.md](security-suites-and-composed-api.md)
   lists the new probes and the `compose_api` move in the backend template.

## Reference

- [authoring-sql-migrations.md](authoring-sql-migrations.md): how to write a migration that is safe on a database
  serving traffic.
- [backend-dockerfile-generation.md](backend-dockerfile-generation.md): generated backend Dockerfiles.
