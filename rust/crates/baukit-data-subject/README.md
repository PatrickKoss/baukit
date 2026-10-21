# baukit-data-subject

Data subject v1 wire types, identifier normalization and Ed25519 DSSE receipts.
The core has no HTTP or database dependencies. The default `server` feature adds
public discovery, authenticated erasure and opt-in export routes, a PostgreSQL
operation store and a durable worker.

```toml
[dependencies]
baukit-data-subject = { version = "=0.10.31", default-features = false }
serde_json = "1"
```

Verify a receipt using discovery obtained from the trusted service:

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

Match the returned service, operation ID, kind and subject reference against the
operation you requested. `ReceiptSigner::from_pkcs8_pem` and `from_pkcs8_der`
load a product's Ed25519 private key. `receipt_key()` returns its current public
discovery entry; `sign()` signs a validated `ReceiptPayload`.

`normalize_request` checks discovery limits and accepted types. Keep its result
transient and store only an HMAC of its serialized normalized body. Raw and
normalized identifier debug output is redacted. Validation errors expose safe
codes without retaining rejected input.

See the [HTTP contract](../../../docs/platform/data-subject-contract.md)
for paths, authentication, status semantics, export parts and exact receipt
encoding. The [public test fixtures](../../../fixtures/data-subject/v1/README.md)
include independent OpenSSL signatures. Never deploy their test private key.

## Mounting the erasure server

Copy the jobs migrations, both erasure migrations and this crate's
`migrations/0001_data_subject.sql` and `migrations/0002_data_subject_exports.sql`
into the product's migration sequence, in that order. `0001_data_subject.sql`
adds a trigger to `job_outbox` and references
`erasure_operations`, so it must run after the `baukit-jobs` and `baukit-erasure`
migrations. `POSTGRES_MIGRATION_SQL` and `POSTGRES_EXPORTS_MIGRATION_SQL` expose
these migrations. Apply both even for erasure-only products. Apply
migrations at deploy time. All stores below must point to the same database.
Products grant their application role access to `data_subject_operations` and
`data_subject_export_parts` like any other table.

Implement `DataSubjectProduct` against the product's tables. `resolve` receives
transient normalized identifiers and returns opaque internal subject IDs.
`erase` deletes owned database records and returns safe category counts and
legal restrictions. Both methods use the supplied connection, do not commit and
do not contact external systems. `erase` must tolerate an already absent record.

The example takes a real product adapter and a configured `baukit-auth` verifier.
It uses retain mode for a shared identity account:

```rust,no_run
use std::sync::Arc;
use axum::{Router, middleware};
use baukit_auth::{AuthState, establish_principal};
use baukit_config::Secret;
use baukit_data_subject::{
    DataSubjectConfig, DataSubjectHandler, DataSubjectProduct, IdentifierType,
    PostgresDataSubjectStore, ReceiptSigner, Service,
    authenticated_routes, discovery_routes,
};
use baukit_erasure::{ErasureService, IdentityRetention, PostgresErasureStore};
use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
use sqlx::PgPool;

fn mount(
    pool: PgPool,
    auth: AuthState,
    product: Arc<dyn DataSubjectProduct>,
    signer: ReceiptSigner,
    hash_key: Secret<String>,
    fence_hash_key: Secret<String>,
) -> Result<(Router, WorkerRunner, PostgresDataSubjectStore), Box<dyn std::error::Error>> {
    let erasure = ErasureService::new(
        PostgresErasureStore::new(pool.clone(), fence_hash_key)?,
        IdentityRetention::Retain,
    )?;
    let config = DataSubjectConfig::new(
        Service { name: "billing".into(), version: "4.2.0".into() },
        ["privacy-officer".into()].into_iter().collect(),
        vec![IdentifierType::Email, IdentifierType::AccountId],
        signer,
        hash_key,
        erasure,
    );
    let store = PostgresDataSubjectStore::new(pool.clone(), config)?;
    let protected = authenticated_routes(store.clone(), product.clone())
        .layer(middleware::from_fn_with_state(auth, establish_principal));
    let app = baukit_http::layers(
        protected.merge(discovery_routes(store.clone())),
        baukit_http::HttpOptions::default(),
    );
    let worker = WorkerRunner::new(
        Arc::new(PostgresJobStore::new(pool)),
        Arc::new(DataSubjectHandler::new(store.clone(), product)),
        WorkerConfig::default(),
    )?;
    Ok((app, worker, store))
}
```

Configure `PrincipalClaimMapping::client_id_claim` for the provider's machine
client claim. Tokens need `data-subject.erase` and an allowed client ID. API
tokens cannot call these routes. Discovery stays outside authentication and
advertises erasure and any configured export support. Serve `app` through the
product's HTTP runtime and run
`worker.run(shutdown)` through its worker supervision.

Keep both hash keys stable with at least 32 bytes of secret material each.
Load the signer from the product's secret configuration. Before constructing
the store, set `previous_receipt_keys`, `limits`, `operation_retention` or
`max_attempts` on the config when the product needs different defaults. Set
`identity_deletion_timeout` to change the wait for identity deletion after local
erasure. It defaults to 24 hours and must be positive and no longer than 30 days.
Previous keys must have `ReceiptKeyStatus::Previous` and distinct valid key IDs.
Default operation retention is 400 days and the default maximum is five attempts.

POST resolves identifiers and commits a pending operation and job together.
The job stores only internal subject IDs, erases all matched subjects in one
transaction, applies their authentication fences and removes its execution
payload. Retain mode signs the result in that transaction. Delete mode waits
for provider completion before signing. No match still produces a signed
`notFound` result.
Legal obligations produce `restricted`. A failed attempt rolls back all subjects;
exhaustion yields `failed` with `data_subject_operation_failed` and no receipt.
Resolver failures happen before acceptance and return a safe `internal` error.

Authentication fences work only when product requests call
`baukit_erasure::reject_fenced_subject` and profile creation calls
`PostgresErasureStore::guard_subject`. Use the same erasure hash key as the
product's self-service profile erasure. Retain mode permits a new empty profile
only after a fresh login strictly after the fence and no unfinished erasure.

For delete mode, pass `IdentityRetention::Delete` with the product's provider
adapter, provider ID, timeout and maximum attempts. Also register the existing
`baukit_erasure::IdentityDeletionHandler` in a worker. The data subject transaction
queues that handler without calling the provider inline. The data subject
operation stays pending without a receipt until every linked identity deletion
completes. The handler's `data-subject.finalize` job checks after one second,
then doubles the delay up to 60 seconds. Its payload holds only the operation ID
and check count. No check is scheduled beyond the deadline set by
`identity_deletion_timeout`, measured from the end of local erasure and recorded
in that transaction. Finalization signs the result in a separate transaction.
Provider failure or an expired
deadline yields a failed data subject operation without a receipt; product
records stay erased and sessions stay fenced. Identity deletion jobs remain
queued after a timeout. Later provider success cannot change the failed data
subject operation. Retain and monitor failed identity jobs for operator repair.

Call `store.purge_expired(chrono::Utc::now(), 1_000).await?` in a product recurring
job. Use `baukit_jobs::FixedUtcInterval` for daily slots, include the slot
identifier in that job's idempotency key, and enqueue its next slot through
`PostgresJobStore::enqueue_in_transaction`. Each purge call deletes at most the
specified number of expired operations and their execution and finalization
jobs. It also deletes at most that many expired export parts independently of
operation retention. Its return value counts operations. It keeps unexpired
operations and receipts, identity fences and identity deletion jobs.

## Enabling exports

Erasure-only products still implement `DataSubjectProduct` without an export
method. Add the separate `DataSubjectExportProduct` implementation to the
product's existing adapter, here named `BillingProduct`, and enable it before
constructing the store:

```rust,no_run
use std::sync::Arc;
use baukit_data_subject::{
    DataSubjectConfig, DataSubjectExportProduct, DataSubjectFuture, ExportCategory,
    LocalSubject,
};
use sqlx::PgConnection;

impl DataSubjectExportProduct for BillingProduct {
    fn export<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a LocalSubject,
    ) -> DataSubjectFuture<'a, Result<Vec<ExportCategory>, sqlx::Error>> {
        Box::pin(async move {
            let records = sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT jsonb_build_object('invoiceId', id, 'total', total) \
                 FROM invoices WHERE subject_id = $1 ORDER BY id",
            )
            .bind(&subject.subject_id)
            .fetch_all(connection)
            .await?;
            Ok(vec![ExportCategory { name: "invoices".into(), records }])
        })
    }
}

fn enable_exports(config: DataSubjectConfig, product: Arc<BillingProduct>) -> DataSubjectConfig {
    let mut config = config.with_export(product);
    config.limits.max_export_part_bytes = 8_388_608;
    config.export_retention = chrono::TimeDelta::days(7);
    config
}
```

Use the same product instance in `with_export`, `authenticated_routes` and
`DataSubjectHandler`. Export uses the configured export product's resolver.
`with_export` requires the export trait at compile time. Without it, discovery
omits export and all export routes return 404. Clients need the allowed machine
identity and `data-subject.export` scope to start, poll or download an export.
An erasure scope alone grants no export access.

The existing handler exports all resolved subjects in one transaction and signs
the part list with the stored bytes. Each part is compact JSON shaped as
`{"category":"invoices","records":[...]}`. Categories sort by name; records
keep product order. Whole records fill each part greedily within the byte limit,
including JSON wrapper bytes. An empty category emits an empty array part.
A record or wrapper that cannot fit fails with
`data_subject_export_record_too_large`, rolls back product work and stores no
parts or receipt. Export does not apply erasure fences or delete identity accounts.

Download each part at
`GET /api/v1/data-subject/exports/{operationId}/parts/{partNumber}` after completion.
Verify the receipt with `ReceiptVerifier::from_discovery`, then hash each part's
exact response bytes and compare its length and SHA-256 with the signed `parts`
entry. Every part parses independently as JSON. Pending downloads return 409;
known expired parts return 410, including after purge. Unknown numbers return 404.
Receipts remain available until operation retention ends.
