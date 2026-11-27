# Migration guides

Each guide covers one breaking or behaviour change and says what a product has to do. Read the ones for the
crates and packages your product uses.

## Upgrading from 0.31.0

- [erasure-subject-rule.md](erasure-subject-rule.md): erasure rejects subjects with control characters at the
  call, and the worker fails operations itself. Copy the new `baukit-erasure`
  `0003_erasure_operation_id_guard.sql` after 0002.

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
