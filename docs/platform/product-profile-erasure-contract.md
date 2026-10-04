# Product-profile erasure contract

**Status:** Contract and conformance boundary.
**Applies to:** Authenticated products that erase a user's product data.
**Related:** [local-data ownership](./local-data-ownership-contract.md),
[analytics privacy](./analytics-privacy-contract.md), and
[`@baukit/api-runtime`](../../typescript/packages/api-runtime/README.md).

Product-profile erasure deletes the product's user data and its identity-provider
account. `baukit-erasure` coordinates the database transaction, receipt, fence,
and durable identity deletion. The product supplies its owned-row deletion
callback and its inventory. The client composition remains in
`@baukit/data-contracts`.

The database transaction is the acceptance boundary. A failure before commit
leaves both systems unchanged. After commit, the product rows are gone and the
identity deletion is durable. A provider outage can leave identity deletion
pending temporarily. The fence blocks old access tokens and profile recreation
while the worker retries. Failed operations require operator repair and rerun;
they are not instructions for the user to start another deletion.

## 1. Client operation

The client follows this order:

1. Run pre-server hooks, such as push-token unregister. These hooks are best
   effort and run in array order. Record a safe warning for each failure and
   continue to the server request.
2. Request authoritative server erasure with a stable idempotency key. Confirmed
   server success is either completed erasure or durable acceptance of an
   asynchronous operation after database deletion has committed and processor
   work has been registered.
3. After that confirmed server success, close and erase the active local
   partition, including its safe identity-registry entry.
4. Run sign-out in a `finally` path around local deletion. Sign-out therefore runs
   after confirmed server success even when local deletion fails.

A confirmed server failure stops before local deletion and sign-out. The session
and local partition remain available so the user can retry or recover. A timeout,
connection loss, cancellation, or unreadable response is not a confirmed server
failure because the server may already have committed the erasure.

`@baukit/data-contracts` exports
`eraseProductProfile(dependencies: ProductProfileErasureDependencies)`. Its
dependencies are optional `beforeServerErase` hooks and required
`eraseServerProfile`, `eraseLocalPartition`, and `signOut` functions. The helper
does not create an idempotency key, issue HTTP requests, or poll operation status.
The product-owned `eraseServerProfile` adapter performs that work and returns an
`ErasureReceipt`:

```ts
type ErasureReceipt =
  | { readonly operationId: string | null; readonly status: "completed" }
  | { readonly operationId: string; readonly status: "pending" };
```

An operation ID is non-empty when present, and a `pending` receipt always has
one. The helper treats a returned receipt with an invalid status or operation ID
as an ambiguous, unreadable server response. It preserves local data and the
session until the product reconciles the server outcome.

The result is a `ProductProfileErasureResult` with one of these exact `status`
values:

| Status | Result |
| --- | --- |
| `erased` | Server acceptance, local erasure, and sign-out succeeded. The result includes `receipt` and `warnings`; the receipt may be `completed` or `pending`. |
| `server-failure` | `eraseServerProfile` rejected with a failure that was not marked ambiguous. Local erasure and sign-out did not run. |
| `ambiguous` | `eraseServerProfile` rejected with `AmbiguousProductProfileErasureError` or an object whose `code` is `product_profile_erasure_ambiguous`. Local erasure and sign-out did not run. |
| `local-failure` | Server acceptance succeeded but local erasure failed. Sign-out was still attempted. The result includes `signOutError`, which is either a sign-out issue or `null`. |
| `signout-failure` | Server acceptance and local erasure succeeded, but sign-out failed. |

The shared `deletion-outcomes.json` fixture adds two test-policy fields outside
`ProductProfileErasureResult`. `serverRetry` is `retry` for `server-failure`,
`reconcile` for `ambiguous`, and `not-required` for the other outcomes.
`sessionRetained` is `true` for `server-failure`, `ambiguous`, and
`signout-failure`; it is `false` after the other outcomes.

Warnings and errors are `ProductProfileErasureIssue` values. Their `stage` is
`before-server`, `server`, `local`, or `sign-out`. Their `cause` is a bounded
error class name, not the original message. Unknown custom `Error` names become
`Error`, and non-`Error` values become `UnknownError`.

A local failure after server success must remain visible to the product so it can
tell the user that device cleanup is incomplete. The product must not present
that result as a full success. Products normally inject
a small `eraseLocalPartition` adapter that awaits
`ScopedPersistenceLifecycle.eraseActivePartition()` and handles its boolean
result. The lifecycle closes the store, resets user-scoped memory, deletes device
data, and removes the registry entry in that order.

Logs may contain bounded operation state, stable error codes, request IDs, an
opaque receipt ID, and the failed cleanup class. They must not contain email,
provider subject, access tokens, request bodies, resource contents, exported
data, object keys derived from user content, or local record values.

## 2. Ambiguous outcomes and idempotency

Create and retain an idempotency key before the first server request. Reuse the
same key after a timeout or connection loss. Do not erase local data or report
server acceptance until a retry or operation-status lookup confirms completed
erasure or a durably accepted asynchronous operation.

The product adapter must classify a timeout, connection loss, cancellation, or
unreadable response as ambiguous by rejecting with
`AmbiguousProductProfileErasureError`. That client error has the stable code
`product_profile_erasure_ambiguous`. An ordinary rejection is returned as
`server-failure`, so the helper cannot infer ambiguity from an error name such as
`TimeoutError` alone.

The server associates the idempotency key with the authenticated product profile
and erasure request. Reusing the key for the same request returns the same receipt
and the same terminal result. It must not start another erasure operation or
create a replacement profile. Reusing a key for an incompatible request is a
conflict.

An asynchronous operation has an opaque erasure operation ID, receipt ID, and a
safe status endpoint. The client persists enough non-sensitive operation state to
resume reconciliation after a restart. Status values distinguish at least
`pending`, `completed`, and `failed`. Completed receipts are immutable. Operators may repair failed operations and rerun their retained jobs. Status
lookup is authorized for the same identity and must not reveal whether another
user's receipt exists.

Those operation-status values belong to the product's HTTP protocol. They are
separate from `ErasureReceipt.status`, whose values are `completed` and
`pending`, and from the five `ProductProfileErasureResult.status` values above.

## 3. Product-owned HTTP responses and errors

Generated authenticated backends expose `DELETE /me` and
`GET /me/erasures/{operationId}`. Both require the token's subject. DELETE
requires one `Idempotency-Key` header with 16 to 128 visible ASCII characters.
The status endpoint compares the token subject's keyed hash with the receipt;
unknown and foreign operation IDs both return 404.

The server executes this sequence:

1. Lock the idempotency key and subject. Same-subject replay returns the stored
   receipt; another subject's reuse returns 409. A different key cannot start
   another erasure for a fenced subject.
2. In one PostgreSQL transaction, call the product callback to erase every owned
   row and owned job, write a pending operation with keyed subject and key
   hashes, insert a keyed subject fence, and enqueue `identity.account.delete`
   through `PostgresJobStore::enqueue_in_transaction`. The job contains the raw
   subject, provider ID and operation ID. Its idempotency key is the operation
   ID. Any failure rolls back all four writes and the product deletion.
3. After commit, try identity deletion once with a short timeout. A provider 404
   succeeds. On success, update the receipt and delete the job together, then
   return 200. On failure, return 202 and let the worker retry with backoff.
4. The worker completes the same receipt and deletes the job in one transaction.
   Permanent failures and exhausted attempts mark the operation failed. This
   includes timeouts and expired final leases. Retain failed jobs for operator
   rerun, exclude them from general retention cleanup, and alert on failed
   operations. The fence remains active during repair.

The inline attempt locks the unclaimed job row and holds one pooled connection
while it calls the provider. This prevents a worker from claiming the same job.
Keep `inline_timeout` short; the template uses three seconds. The timeout covers
the entire provider call, including token acquisition and waiting for the token
cache, rather than giving token acquisition and deletion separate budgets.
Inline failures log the operation ID and error class without the subject.
Database completion failures also log their class and leave the durable job for
worker reconciliation.

Generated auth backends run a supervised identity runner inside the API in both
flavors, with or without a separate worker. The optional worker handles the
item-created demo jobs. Each runner claims only its handler's job types, so the
two runners do not claim each other's jobs. Deploy the API while identity
erasures remain pending and alert if its runner fails.

A 200 body is
`{"status":"completed","operationId":"<uuid>","completedAt":"<RFC 3339>"}`.
A 202 body is `{"status":"pending","operationId":"<uuid>"}` with
`Location: /me/erasures/<operationId>`. DELETE replay keeps the operation ID.
A pending receipt follows the operation's current state, including completion
and its stable completion timestamp. A failed DELETE replay returns 200 with
`{"status":"failed","operationId":"<uuid>"}` and no `completedAt`.
The HTTP code confirms that the stored operation was read. Its body reports the
failure. Only a pending operation returns 202.

`createProfileErasureClient().erase()` rejects a failed receipt with
`ProfileErasureOperationFailedError`, code `erasure_operation_failed`, and the
existing `operationId`. This is a known failure, not an ambiguous transport
outcome. The client keeps the durable idempotency key. Reconciliation after
operator repair reuses that key and reads the same operation. `poll()` instead
returns `{ status: "failed", operationId }` and stops polling.

The UI must say that product rows are already erased and identity deletion
needs support. Keep the operation ID available for support and later status
checks. A retry may reconcile the same operation after repair; it must not
start a new deletion or claim that the earlier transaction rolled back.
Operators repair and rerun the retained job. The subject fence stays active.
The local erasure helper treats this rejection as `server-failure` and preserves
the session and local partition until the product resumes reconciliation.
The product adapter must save the error's operation ID before passing the
rejection to that helper, whose result contains only a bounded error class.

`fixtures/erasure/receipts-v1.json` pins pending, completed and failed DELETE
status/body pairs. Both Rust and TypeScript tests consume it.

GET returns 200 with status `pending`, `completed` or `failed`, the operation ID,
and `completedAt` only after completion. Every ordinary authenticated request
from a fenced subject returns 401 `profile_erased`. Status lookup and DELETE
reconciliation are allowed without resolving a profile. Subject resolution
checks the fence under the same transaction lock used by erasure before any
profile insert. Checking only in HTTP middleware leaves a creation race.

Fences contain only HMAC-SHA256 subject hashes. Operations contain only keyed
hashes, timestamps and safe response bodies. Completed job rows are deleted
immediately, so no row retains the raw subject after completion. Keycloak
subjects are UUIDs and are not reused; fences may remain permanently.

### Keycloak account deletion

`KeycloakAccountDeleter` uses client credentials for the backend confidential
client in the product realm. Grant its service account
`realm-management/manage-users`. Keycloak has no delete-only role; this grant
also permits other user-management operations. Reconciliation applies that role
mapping without rotating the existing client secret.

Deletion calls `DELETE {base}/admin/realms/{realm}/users/{subject}`. The OIDC
subject is Keycloak's user ID. Deleting the account ends its sessions. The admin
client caches tokens until shortly before expiry. Network failures, 5xx, 429 and
token failures are retryable. A 403 from the admin deletion endpoint or a 400 is
permanent and requires operator action.

The admin client has connect and request timeouts, disables redirects and
proxies, and requires HTTPS. An explicit local-development allowance permits
HTTP. It has its own reqwest client because `baukit-egress` blocks private
addresses and Keycloak is commonly in-cluster.

Load the client secret and hash key through `baukit_config::Secret`. Generated
local development uses the realm's local backend secret and a local hash key.
Other environments require configured secrets and HTTPS. Provision at least 32
random bytes for the hash key. Keep it stable and back it up with deployment
secrets. Changing it without migrating existing hashes invalidates fences and
receipt authorization. Never log credentials, tokens or subjects.

Every HTTP error response uses the standard envelope emitted by `baukit-http`
and parsed by `@baukit/api-runtime`:

```json
{
  "error": {
    "code": "product_profile_erasure_failed",
    "message": "The product profile could not be erased",
    "requestId": "...",
    "details": {}
  }
}
```

The minimum stable product code set is:

| Code | Meaning |
| --- | --- |
| `erasure_idempotency_key_invalid` | The required key is missing, repeated or malformed. |
| `profile_erased` | A fenced subject cannot access product data or recreate a profile. |
| `unauthenticated` | A valid authenticated principal is required. |
| `permission_denied` | The principal may not erase the addressed profile. |
| `erasure_idempotency_conflict` | The key was already used for an incompatible request. |
| `erasure_operation_not_found` | The receipt is unknown or is not visible to this principal. |
| `product_profile_erasure_failed` | Authoritative erasure definitively failed without a success result. |
| `erasure_operation_failed` | An accepted asynchronous erasure reached a terminal failure. |

Codes are stable snake_case values. `message` is safe fallback copy, `details` is
a JSON object with safe structured values, and `requestId` connects the response
to internal diagnostics. Clients localize `code` plus `details` and use `message`
only as a fallback. Internal causes and processor payloads never cross the API.
Products document these responses in OpenAPI.

A transport timeout, including a `request_timeout` response when the server
cannot prove rollback, remains ambiguous. The client reconciles it with the same
idempotency key or the status endpoint. It does not translate a transport error
into `product_profile_erasure_failed` on its own.

## 4. Mandatory deletion inventory

Every product maintains an inventory that answers how and when each class is
removed:

- relational rows and soft-delete tombstones;
- outbox, inbox, retry, and scheduled job payloads;
- object storage and generated exports;
- device partitions, caches, files, and identity registry entries;
- push registrations and external integration credentials;
- analytics deletion or a documented retention policy;
- backups and the point at which natural expiry removes the data;
- the identity-provider account and sessions, removed by the identity deletion job.

The confirmation UI and receipt use this inventory to make accurate claims.
Asynchronous processors include their expected completion or retention period.
Backup entries state the maximum time until expiry and whether restored backups
reapply erasure records before serving data.

## 5. Backend conformance

Each product maintains an owned-resource registry. Its entries follow the
`OwnedResourceCheck` struct: `name: &'static str`, `count_sql: &'static str`, and
`cleanup: CleanupKind`. The enum variants are `Cascade`, `Explicit`, and
`AsyncProcessor`. `Cascade` declares database cascade deletion, `Explicit`
declares deletion in product erasure code, and `AsyncProcessor` declares deletion
by a registered background processor. The main harness passes each entry to the
product adapter. It does not execute `count_sql` itself.

Products implement `ProductProfileErasureAdapter` with these methods:
`seed_user_owned_resource_graph`, `owned_resource_count`,
`erase_product_profile`, and `registered_background_job_count`. An adapter may
also return a non-empty `unseeded_resource_reason` when its fixture cannot create
a row for one `Cascade` or `Explicit` entry. The adapter error type must implement
`Error + Send + Sync + 'static`; its message is not included in harness output.

`check_product_profile_erasure_conformance(adapter, subject, resources)` returns
`Result<(), ErasureConformanceError>`. It reports an empty subject or registry,
empty resource names, duplicate resource names, empty count SQL, and count SQL
without the `$1` subject binding as violations. It then seeds once and counts
every registered resource and the aggregate registered-background-job count
before erasure. Every `Cascade` and `Explicit` entry must have at least one row
unless the adapter supplies its reason, and at least one registered resource
must be nonzero. The harness then invokes erasure and checks every resource and
the aggregate registered-background-job count for zero. It invokes erasure a
second time and checks the same counts again. The repeated invocation tests that
the adapter is safe to call twice. `ErasureConformanceError.violations()` exposes
the bounded violation strings. If an asynchronous processor must finish before
zero can be observed, the product adapter waits or polls inside
`erase_product_profile`; the harness has no processor-status or waiting API.

The product-owned graph includes relational descendants, tombstones, jobs, and
each registered resource class. A sampled happy-path graph is not sufficient if
the registry omits a user-owned resource class.

With the `sqlx-postgres` feature,
`audit_user_root_foreign_keys(pool, user_root_table, resources)` inspects direct
foreign keys to the product's user root and returns
`Result<Vec<ForeignKeyDeleteMismatch>, sqlx::Error>`. A registry entry declared
`Cascade` must use `ON DELETE CASCADE`. An unregistered direct reference is also
treated as `Cascade`, so a non-cascading action is returned as a mismatch.
Entries declared `Explicit` or `AsyncProcessor` are accepted with any database
delete action. Each mismatch contains `constraint_name`, schema-qualified
`referencing_table`, `actual_delete_action`, and `declared_cleanup`.

Resources without a direct foreign key, including job payloads and external
systems, remain mandatory registry entries and cannot be discovered by this
audit alone.

Conformance runs with isolated test identities and reports resource names and
counts only. Failure output and database diagnostics must not print inserted user
content, credentials, request bodies, or processor payloads.

Products also implement `IdentityErasureAdapter` and run
`check_identity_erasure_conformance` through their real authenticated endpoint
and registered worker wiring. It injects one provider failure, checks that
product rows are erased while the receipt and job remain pending, verifies
replay and 409 conflict, rejects fenced profile resolution, runs the worker,
and checks completed replay plus the absence of every raw-subject location.
Use `FakeIdentityAccountDeleter` for the provider; its calls and scripted errors
are test observations, never log fields.

Analytics deletion is separate external work. Include analytics deletion or a
documented retention policy in the inventory and follow the
[analytics privacy contract](./analytics-privacy-contract.md). Completing the
identity job alone does not prove that analytics, object storage or exports have
been removed. Register those processors durably before accepting erasure, and
make completion copy reflect their state.

## 6. Acceptance checks

- Pre-server hook failure produces a warning and does not block the authoritative
  request.
- Confirmed server failure preserves the session and local partition.
- Timeout and lost-response cases reuse the idempotency key and reconcile before
  any local deletion.
- Immediate success and asynchronous success erase the local partition, remove
  its registry entry, and attempt sign-out in the required order.
- Local deletion and sign-out failures remain distinguishable in the result.
- Repeated requests return the same receipt and terminal result.
- Confirmation copy says the identity-provider account will also be deleted.
  Pending copy describes outstanding provider or other processor work.
- The deletion inventory addresses all eight classes, records `not applicable`
  where needed, and the conformance graph reaches zero for every registered
  resource.
- Direct user-root foreign keys declared `Cascade`, and unregistered direct
  foreign keys, fail the schema audit unless their database action is
  `ON DELETE CASCADE`.
- Tests and logs contain no user content or credentials.

## 7. Adopting this in an existing product

- Add `baukit-erasure` and migrate the existing receipt table to the shared
  operation store. Preserve receipt IDs, replay responses and identity ownership
  during migration. Apply the job failure trigger after the jobs schema.
- Move all owned-row and owned-job deletion into `ProductErasure`. Include API
  token deletion and every entry in the product's inventory.
- Add fence checks to authenticated requests. Lock and check the fence inside
  subject resolution before inserting a profile.
- Register `IdentityDeletionHandler` with the durable worker. Alert on failed
  operations and provide a repair-and-rerun procedure that retains their jobs.
- Configure the confidential admin client and permanent keyed-hash secret.
  Grant and reconcile the backend service account's `manage-users` role.
- Expose the 200/202 DELETE receipt and subject-authorized status endpoint.
  Keep the same idempotency key across ambiguous responses.
- Change UI copy that says the IdP account stays. Show pending work honestly,
  then run identity-erasure conformance through the product's actual router.
