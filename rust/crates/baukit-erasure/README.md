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
same `sub` from accessing this product or recreating its profile afterwards;
other apps can still use the shared account.

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
`baukit-jobs` migrations. Keep the hash key in `baukit_config::Secret<String>`.
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
transaction to prevent races with erasure.

`KeycloakAccountDeleter` uses the backend confidential client's service account.
Grant `realm-management/manage-users`, which allows more than deletion because
Keycloak has no delete-only role. It uses HTTPS, bounded timeouts, no proxy and
no redirects. Set `allow_local_http` only in local development.

See [the erasure contract](../../../docs/platform/product-profile-erasure-contract.md)
for HTTP responses and the product deletion inventory.
