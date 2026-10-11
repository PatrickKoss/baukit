# baukit-erasure

Erase product rows and choose whether to delete the identity provider account.
Pass an explicit `IdentityRetention` to `ErasureService::new`.

Use `IdentityRetention::Retain` when several apps share one provider account,
such as a Clerk instance with satellite domains:

```rust
let service = ErasureService::new(store, IdentityRetention::Retain)?;
```

This mode commits product erasure, a completed receipt with `completedAt`, and
the subject fence in one transaction. DELETE returns 200, and status lookup and
same-key replay return that completed receipt. No identity deletion job or
provider call runs. The provider account and sessions stay. The fence keeps the
old sessions from accessing this product. After completion, the same `sub` can
log in again and create a fresh, empty profile. Other apps keep their shared
account.

Use `IdentityRetention::Delete` when erasure should also delete the provider
account:

```rust
let service = ErasureService::new(
    store,
    IdentityRetention::Delete {
        deleter,
        provider_id: "clerk".into(),
        inline_timeout: Duration::from_secs(3),
        max_attempts: 12,
    },
)?;
```

`ErasureService::erase` calls the product's `ProductErasure` implementation on
that transaction, writes a keyed receipt and fence, and enqueues
`identity.account.delete`. After commit it tries the provider once with a bounded
timeout. Keep `inline_timeout` short: the call holds a job row lock and one pool
connection. Its budget includes token acquisition and deletion. Inline failure
warnings contain the operation ID and error class without the subject.
Success returns a completed receipt; failure returns durable acceptance.

Copy `POSTGRES_MIGRATION_SQL` into product migrations after the three
`baukit-jobs` migrations. Copy `POSTGRES_REONBOARDING_MIGRATION_SQL` after 0001
erasure. It adds `erased_at TIMESTAMPTZ NOT NULL DEFAULT now()` to the fences.
Keep the hash key in `baukit_config::Secret<String>`.
Use at least 32 random bytes and keep that key stable across deployments. Losing
or replacing it breaks receipt authorization and fences.

In delete mode, register `IdentityDeletionHandler` with a `WorkerRunner`.
Permanent failures, exhausted attempts, timeouts and expired final leases mark
the operation failed.
Alert on failed operations. Repair credentials or permissions, then reset the
job's attempts and status to pending to rerun it. Never delete failed identity
jobs through general retention cleanup. The fence remains active. Successful
completion deletes the job payload in the same transaction as the receipt update.

Call `reject_fenced_subject` after authentication for ordinary requests. Allow
status lookup and same-key DELETE replay without resolving a profile. Profile
resolution must call `PostgresErasureStore::guard_subject` inside its own insert
transaction to prevent races with erasure. Pass `principal.authenticated_at()`
to both checks and `ErasureService::erase`. `is_fenced(subject, authenticated_at)`
returns whether that session is rejected.

A fenced subject is admitted only when its verified `auth_time` is present,
strictly later than `erased_at`, and every operation for that subject is
`completed`. Missing login times, equal or earlier times, pending operations,
and failed operations return 401 `profile_erased`. Unfenced subjects need no
login time. A new erasure key updates `erased_at` with the transaction's `now()`
and fences the re-onboarded session again. Earlier keys still return their stored
receipts and never erase the new profile.

After `profile_erased`, shared SSO clients must force a fresh login with
`prompt=login`. Silent SSO and refresh keep the old `auth_time`. The auth-web and
auth-native adapters use `clearSession({ reason: 'profile_erased' })` to request
that fresh login. Clerk and WorkOS need a JWT template that adds the session's
original numeric `auth_time`, unchanged on refresh. Without it, re-onboarding
stays unavailable. Never substitute `iat`, which changes on refresh.

The database sets `erased_at`; the application never supplies that clock value.
Clock skew fails closed only when the provider clock is behind the database:
a fresh login may still appear too old. If the provider clock is ahead, a
pre-erasure session can appear newer than the fence and be admitted after
completion. The verifier's clock leeway bounds future claims against the app
clock, not against the database clock. Keep provider, app and database clocks
synchronized.

`KeycloakAccountDeleter` uses the backend confidential client's service account.
Grant `realm-management/manage-users`, which allows more than deletion because
Keycloak has no delete-only role. It uses HTTPS, bounded timeouts, no proxy and
no redirects. Set `allow_local_http` only in local development.

See [the erasure contract](../../../docs/platform/product-profile-erasure-contract.md)
for HTTP responses and the product deletion inventory.
