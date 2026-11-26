# Erase user data and answer data subject requests

At the end of this page your product deletes a user's data on request, decides whether the
identity provider account goes too, answers machine-to-machine data subject requests with
signed receipts and keeps analytics behind consent. The page is for product developers who
ship a product with sign-in.

Two crates do the server work. `baukit-erasure` handles self-service profile erasure and
the fence that keeps old sessions out. `baukit-data-subject` adds the data subject API and
DSSE receipts on top of it. The contracts define the wire behaviour. This page shows how
to put the pieces together.

## What the template already does

A product generated with `--auth` already uses `baukit-erasure`. The template wires:

- `identity_erasure` in `templates/backend/backend/crates/__app__-bin/src/lib.rs`, which
  builds a `PostgresErasureStore`, an `IdentityDeletionHandler` and an `ErasureService`
  with `IdentityRetention::Delete`. It picks `KeycloakAccountDeleter`,
  `ClerkAccountDeleter` or `WorkOsAccountDeleter` from the auth provider.
- `register_identity_provider_readiness`, which reports
  `ErasureService::probe_identity_provider` as the `identity_provider` readiness check.
- the request guard in `templates/backend/backend/crates/__app__-api/src/lib.rs`, which
  calls `reject_fenced_subject` and wraps the request in `with_write_fence`.
- migrations `0004_identity_erasure_jobs.sql`, `0005_identity_erasure.sql` and
  `0006_identity_erasure_reonboarding.sql` under `templates/backend/__auth__/backend/migrations/`.

The data subject API is not in any template. You add it by hand, as shown below.

## Choose an identity retention mode

`ErasureService::new` takes an explicit `IdentityRetention`. There is no default.

Pick `IdentityRetention::Retain` when several apps share one provider account, for example
a Clerk instance with satellite domains. The product rows, a completed receipt and the
subject fence commit in one transaction. No provider call runs. The account and its
sessions stay, the fence keeps old sessions out of this product, and the same `sub` can
sign in again later and get a fresh, empty profile.

Pick `IdentityRetention::Delete` when the account belongs to this product alone. It takes
four values:

- `deleter`, an `Arc<dyn IdentityAccountDeleter>`
- `provider_id`, matching the registered `IdentityDeletionHandler`
- `inline_timeout`, the budget for one inline provider call after commit
- `max_attempts`, for the durable worker

The service enqueues an `identity.account.delete` job (`IDENTITY_DELETE_JOB_TYPE`) in the
erasure transaction and tries the provider once after commit. The template uses 3 seconds
and 12 attempts. Keep the timeout short, because the call holds a job row lock and a pool
connection. Register `IdentityDeletionHandler` with a `WorkerRunner`, and alert on failed
operations. [Run background jobs](jobs.md) covers the worker.

The decision table:

| Situation | Mode |
|---|---|
| One product per provider account | `Delete` |
| Several products share the account | `Retain` |
| The provider has no deleter in baukit | `Retain`, or implement `IdentityAccountDeleter` |

Retain-mode re-onboarding needs a verified `auth_time` claim that does not change on
refresh. Keycloak sends it. Clerk and WorkOS need a JWT template that adds it, or an
erased user stays fenced. The baukit-erasure README explains the clock rules.

## Guard every request

The fence works only when product code checks it:

- Call `reject_fenced_subject(store, subject, authenticated_at)` after authentication on
  ordinary requests. A fenced session gets 401 `profile_erased`.
- Profile creation calls `PostgresErasureStore::guard_subject` inside its insert
  transaction.
- Writes open with `PostgresErasureStore::begin_guarded_write`, or the request runs inside
  `with_write_fence(WriteFence::new(store, subject, authenticated_at), next)` and opens
  writes with `begin_fenced_write(&pool)`. Both hold the subject lock until commit.

Pass `principal.authenticated_at()` to all of them. The template's middleware is the
working example.

`ProductErasure` is the product's callback. `ErasureService::erase` calls it on the
erasure transaction to delete the owned rows. The contract's deletion inventory lists what
it must cover, and other crates' tables belong in it. For push, see
[Send push and local notifications](push-and-notifications.md).

Keep the subject hash key in `baukit_config::Secret<String>`, with at least 32 random
bytes, stable across deployments. To rotate it, build the store with
`PostgresErasureStore::with_retired_keys` and call `rehash_subjects`.

## Erase on the client

`@baukit/data-contracts` exports `eraseProductProfile`. It runs the `beforeServerErase`
hooks, then `eraseServerProfile`, then `eraseLocalPartition` and finally `signOut`. A
confirmed server failure stops before local deletion. A timeout does not count as a
failure, because the server may have committed. The client supplies the idempotency key.
After a `profile_erased` response, shared SSO clients force a fresh login. The auth-web and
auth-native adapters do that with `clearSession({ reason: 'profile_erased' })`.

## Mount the data subject API

`baukit-data-subject` serves these routes:

- `GET /.well-known/baukit/data-subject` from `discovery_routes`, public
- `POST /api/v1/data-subject/erasures` and `GET /api/v1/data-subject/erasures/{operationId}`
  from `authenticated_routes`
- the three `/api/v1/data-subject/exports` routes, only when the config has an export product

The steps:

1. Implement `DataSubjectProduct` against your tables. `resolve` maps normalized
   identifiers to internal subject IDs. `erase` deletes the owned records on the supplied
   connection and does not commit.
2. Build an `ErasureService` with the same erasure hash key the self-service erasure uses.
3. Build a `DataSubjectConfig` with `DataSubjectConfig::new(service, allowed_client_ids,
   identifier_types, signer, hash_key, erasure)`. Load the `ReceiptSigner` with
   `from_pkcs8_pem` or `from_pkcs8_der` from secret configuration.
4. Build a `PostgresDataSubjectStore`, put `authenticated_routes` behind
   `establish_principal` and merge `discovery_routes` outside it.
5. Run a `WorkerRunner` with `DataSubjectHandler`. In delete mode, also register
   `IdentityDeletionHandler`.
6. Call `store.purge_expired(now, limit)` from a recurring job.

The full `mount` function is a compiled doctest in the "Mounting the erasure server"
section of `rust/crates/baukit-data-subject/README.md`. Copy it from there.

Callers are machine clients. Their tokens need the `data-subject.erase` scope, or
`data-subject.export` for exports, and a client ID in the allowed set. Set
`PrincipalClaimMapping::client_id_claim` for your provider's client claim. API tokens
cannot call these routes. Mount the routes inside the product's rate limiter like any other
route.

In retain mode the job signs the receipt in the erasure transaction. In delete mode the
operation stays pending until every linked identity deletion completes, then a
`data-subject.finalize` job signs it. `identity_deletion_timeout` bounds that wait. It
defaults to 24 hours.

To add exports, implement `DataSubjectExportProduct` on the same adapter and call
`DataSubjectConfig::with_export` before building the store. The README's "Enabling exports"
section has the code.

## Verify a receipt

Callers verify receipts against discovery from the trusted service. From
`rust/crates/baukit-data-subject/README.md` (doctest):

```rust
use baukit_data_subject::{DiscoveryDocument, DsseEnvelope, ReceiptPayload, ReceiptVerifier};

fn verify(discovery_json: &[u8], envelope_json: &[u8])
    -> Result<ReceiptPayload, Box<dyn std::error::Error>>
{
    let discovery: DiscoveryDocument = serde_json::from_slice(discovery_json)?;
    let envelope: DsseEnvelope = serde_json::from_slice(envelope_json)?;
    let verifier = ReceiptVerifier::from_discovery(&discovery)?;
    Ok(verifier.verify(&envelope)?)
}
```

Then match the service, operation ID, kind and subject reference against the request you
sent. The fixtures in `fixtures/data-subject/v1/` include signatures made with OpenSSL.

## Apply the migrations in order

Migrations from several crates touch each other's tables. `rust/migrations.toml` records
the order, and `scripts/check-migrations.py` fails when a crate comes before one it names
in `after`. For privacy, the order is:

1. `baukit-jobs`, `0001` to `0004`. `0001_erasure.sql` adds a trigger on `job_outbox`.
2. `baukit-erasure`, `0001_erasure.sql` (`POSTGRES_MIGRATION_SQL`) and
   `0002_erasure_reonboarding.sql` (`POSTGRES_REONBOARDING_MIGRATION_SQL`).
3. `baukit-data-subject`, `0001_data_subject.sql` (`POSTGRES_MIGRATION_SQL`) and
   `0002_data_subject_exports.sql` (`POSTGRES_EXPORTS_MIGRATION_SQL`). `0001` adds a
   trigger to `job_outbox` and references `erasure_operations`. Apply both files even for an
   erasure-only product.

`baukit-erasure` and `baukit-data-subject` both list `after = ["baukit-jobs"]`. Apply
`baukit-erasure` before `baukit-data-subject`, because the data subject SQL references
the erasure tables. Grant the application role access to `data_subject_operations` and
`data_subject_export_parts`. A role with column privileges also needs
`GRANT UPDATE (erased_at) ON erasure_fences`.

## Gate analytics on consent

`@baukit/analytics-core` starts with consent `unknown` and drops events until the product
calls `setConsent('granted')`. `setConsent('denied')` discards every unsent command and
calls the transport's `clearPending`. Identity uses the internal user UUID, never the
provider subject or email. The analytics privacy contract lists the forbidden properties
and the scrubber rules. When a user erases their profile, delete their person and events
in the analytics provider as part of the product's deletion runbook.

## Reference

- [baukit-erasure README](../../rust/crates/baukit-erasure/README.md)
- [baukit-data-subject README](../../rust/crates/baukit-data-subject/README.md)
- [Product-profile erasure contract](../platform/product-profile-erasure-contract.md)
- [Data subject contract](../platform/data-subject-contract.md)
- [Analytics privacy contract](../platform/analytics-privacy-contract.md)
- [Local-data ownership contract](../platform/local-data-ownership-contract.md)
- [`@baukit/data-contracts` README](../../typescript/packages/data-contracts/README.md)
- [`@baukit/analytics-core` README](../../typescript/packages/analytics-core/README.md)
- [Data subject test fixtures](../../fixtures/data-subject/v1/README.md)
- [Data subject client credentials migration guide](../migrations/data-subject-client-credentials.md)
- [Order SQL migrations](../migrations/authoring-sql-migrations.md)
- [ADR 0025: erasure and data subject requests](../adr/0025-erasure-and-data-subject-requests.md)
- [ADR 0021: job ownership keys for erasure](../adr/0021-job-ownership-keys-for-erasure.md)
- [ADR 0026: reference SQL migrations](../adr/0026-reference-sql-migrations.md)
