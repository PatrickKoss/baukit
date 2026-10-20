# Data subject contract implementation plan

**Status:** In progress. Claude orchestrates; codex agents (gpt-6.1-sol, xhigh) implement in `ds-<task>` worktrees on branches cut from `feature/data-subject`. Claude reviews every diff, merges into `feature/data-subject`, runs the CI-equivalent gate, then merges to `main` and cuts a release train.

**Prepared:** 2026-10-20

**Baukit baseline:** `d531d998`, release `0.10.31`

**Consumer:** Runtime Analyzer `docs/datenwache-plan.md`, tasks P6.3 to P6.5 (dependencies B1 to B3).

## Why

`baukit-erasure` covers one case: a signed-in user deletes their own profile through `DELETE /api/v1/me`. A data subject request is different. A privacy officer's tool asks every service of a customer to find a person by identifiers, erase or export what it holds about them, and return proof. Runtime Analyzer orchestrates such requests and seals the proofs into evidence. It needs one contract that every Baukit product can adopt by adding a crate, and that services outside Baukit can implement from a document.

## Scope

- B1: the contract document `docs/platform/data-subject-contract.md`, the crate `rust/crates/baukit-data-subject` with discovery, erasure and signed receipts.
- B2: export operations in the same crate and contract.
- B3: a conformance kit in `baukit-test`. It has two parts: a check a service runs against its own router, and a controlled target server that client implementations such as Runtime Analyzer test against.
- The three open RustSec advisories (RUSTSEC-2026-0381, 0382, 0383), so the release train's `cargo deny` passes.

Out of scope for v1: template and CLI wiring into generated products, a TypeScript client, identity verification of the data subject, rectification and restriction-of-processing requests.

## Contract v1 (binding for every task)

Field names are camelCase. Errors use the existing `baukit-http` error envelope and codes below.

### Discovery

`GET /.well-known/baukit/data-subject`, unauthenticated, cacheable for five minutes:

```json
{
  "contractVersion": "1",
  "service": { "name": "billing", "version": "4.2.0" },
  "operations": ["erasure", "export"],
  "identifierTypes": ["email", "accountId"],
  "scopes": { "erasure": "data-subject.erase", "export": "data-subject.export" },
  "limits": { "maxIdentifiers": 16, "maxIdentifierLength": 256, "maxExportPartBytes": 8388608 },
  "receiptKeys": [
    { "keyId": "<hex sha256 of SPKI DER>", "algorithm": "ed25519", "publicKeyPem": "-----BEGIN PUBLIC KEY-----...", "status": "current" }
  ]
}
```

`receiptKeys` lists the current key and any previous keys that signed receipts still retained. `status` is `current` or `previous`. Exactly one key is `current`.

### Identifiers

A request carries 1 to `maxIdentifiers` identifiers `{ "type": "...", "value": "..." }`. Built-in types are `email`, `accountId`, `customerId` and `phone`. A service may define its own types prefixed with `x-`. Normalization before matching: `email` is trimmed and lowercased, `phone` must be E.164 (`+` and 8 to 15 digits), the others are trimmed and compared exactly. A service rejects types it did not list in discovery with `data_subject_identifier_type_unsupported`. Identifiers are never stored in plain text, never logged and never put into a receipt. The operation store keeps only an HMAC of the normalized request for idempotency conflict detection.

`subjectReference` is an opaque client string of 16 to 128 visible ASCII characters, usually the client's HMAC of the subject. The service echoes it in the operation and the receipt so the client can match proofs without the service learning more.

### Operations

| Method and path | Scope | Purpose |
| --- | --- | --- |
| `POST /api/v1/data-subject/erasures` | `data-subject.erase` | Start an erasure |
| `GET /api/v1/data-subject/erasures/{operationId}` | `data-subject.erase` | Erasure status and receipt |
| `POST /api/v1/data-subject/exports` | `data-subject.export` | Start an export |
| `GET /api/v1/data-subject/exports/{operationId}` | `data-subject.export` | Export status, parts and receipt |
| `GET /api/v1/data-subject/exports/{operationId}/parts/{partNumber}` | `data-subject.export` | One bounded JSON part |

`POST` requires `Idempotency-Key` with 16 to 128 visible ASCII characters and a body `{ "subjectReference": "...", "identifiers": [...] }`. A replay with the same key and the same normalized body returns the same operation. The same key with a different body returns 409 `data_subject_idempotency_conflict`. `POST` returns 202 with a `pending` operation, or 200 for a replay of a terminal one. Clients poll `GET` until the status is terminal.

The operation resource:

```json
{
  "operationId": "uuid",
  "kind": "erasure",
  "status": "pending",
  "subjectReference": "...",
  "createdAt": "rfc3339",
  "completedAt": null,
  "categories": [],
  "restrictions": [],
  "parts": [],
  "receipt": null,
  "error": null
}
```

Statuses: `pending`; terminal `completed`, `notFound`, `restricted`, `failed`. `restricted` means the service erased what it could and keeps the listed categories under a legal obligation (Art. 17(3) GDPR). Each restriction is `{ "category", "reason", "legalBasis", "retainUntil" }` with `retainUntil` a date or null. `categories` lists `{ "category", "action": "erased" | "exported" | "retained", "count" }`. `failed` carries `error.code` and no receipt; every other terminal status carries a receipt. `notFound` is a signed statement that no identifier matched. For a completed export, `parts` lists `{ "partNumber", "sha256", "bytes" }` and each part is downloadable until it expires (410 `data_subject_export_expired`). Before completion a part request returns 409 `data_subject_export_not_ready`.

### Receipts

A receipt is a DSSE envelope `{ "payloadType", "payload", "signatures": [{ "keyid", "sig" }] }`. `payload` and `sig` are standard base64. `payloadType` is `application/vnd.baukit.data-subject.receipt.v1+json`. The signature is Ed25519 over the DSSE v1 pre-authentication encoding `"DSSEv1" SP len(type) SP type SP len(body) SP body`. `keyid` is the lowercase hex SHA-256 of the public key's SPKI DER, which is how Runtime Analyzer already names its keys. Verifiers check the signature over the payload bytes as received and never re-serialize.

Receipt payload:

```json
{
  "receiptType": "baukit.data-subject.receipt.v1",
  "contractVersion": "1",
  "operationId": "uuid",
  "kind": "erasure",
  "status": "completed",
  "subjectReference": "...",
  "service": { "name": "billing", "version": "4.2.0" },
  "createdAt": "rfc3339",
  "completedAt": "rfc3339",
  "categories": [{ "category": "invoices", "action": "erased", "count": 3 }],
  "restrictions": [],
  "parts": []
}
```

### Auth

Callers are machines. The product's existing `baukit-auth` middleware establishes a `Principal` from an OAuth 2.0 client-credentials access token issued by the product's identity provider. The data subject routes accept a principal only if its `client_id()` is in the configured allowlist, it is not an API token, and `scopes()` contains the operation's scope. No principal returns 401; a principal that is not allowed returns 403 `data_subject_client_not_allowed`; a missing scope returns 403 `insufficient_scope`. A client allowed for one operation cannot read the other kind's operations. Operations are visible only to the client that created them.

### Errors

| Code | Status | Meaning |
| --- | --- | --- |
| `data_subject_idempotency_key_invalid` | 400 | Missing or malformed `Idempotency-Key` |
| `data_subject_request_invalid` | 422 | Bad `subjectReference`, empty or too many identifiers, value too long, phone not E.164 |
| `data_subject_identifier_type_unsupported` | 422 | Type not listed in discovery |
| `data_subject_idempotency_conflict` | 409 | Same key, different request |
| `data_subject_operation_not_found` | 404 | Unknown operation, other kind, or another client's |
| `data_subject_export_not_ready` | 409 | Part requested before completion |
| `data_subject_export_expired` | 410 | Part past its retention |
| `data_subject_client_not_allowed` | 403 | Principal is not an allowed machine client |
| `insufficient_scope` | 403 | Scope missing |

### Retention

Export parts expire after a configurable period, default seven days. Operation rows and receipts are purged after a configurable period, default 400 days, so a client can re-fetch a receipt within the legal answer window and its extensions.

## Crate design

`rust/crates/baukit-data-subject`, hexagonal like the other crates:

- Always compiled, no Axum or SQLx: contract types, identifier validation and normalization, receipt payload, DSSE signing and verification, key id derivation. A client such as Runtime Analyzer depends on this part with `default-features = false`.
- Feature `server` (default): Axum router, PostgreSQL operation store with migrations in `migrations/`, a `baukit-jobs` handler that runs pending operations, the purge job.
- The product implements one trait. Each method runs inside the operation's transaction, must not call external systems and must not commit:

```rust
pub trait DataSubjectProduct: Send + Sync {
    fn resolve<'a>(&'a self, connection: &'a mut PgConnection, identifiers: &'a [NormalizedIdentifier])
        -> DataSubjectFuture<'a, Result<Vec<LocalSubject>, sqlx::Error>>;
    fn erase<'a>(&'a self, connection: &'a mut PgConnection, subject: &'a LocalSubject)
        -> DataSubjectFuture<'a, Result<ErasureReport, sqlx::Error>>;
    fn export<'a>(&'a self, connection: &'a mut PgConnection, subject: &'a LocalSubject)
        -> DataSubjectFuture<'a, Result<Vec<ExportCategory>, sqlx::Error>>;
}
```

- `LocalSubject` carries the product's subject id and an optional authentication subject. When the authentication subject is set, erasure also applies the `baukit-erasure` fence and identity retention mode in the same transaction, so the person cannot keep using a session. That needs a small public operator-initiated entry point in `baukit-erasure`; it must not change the behaviour of `DELETE /api/v1/me`.
- Signing keys come from the product: one current Ed25519 PKCS#8 private key plus previous public keys.

## Tasks

| Task | Content | Depends on |
| --- | --- | --- |
| D0 | Bump `webpki-roots`, `webpki-root-certs` and `rustls-webpki` so `cargo deny` passes | none |
| D1 | Contract document, crate with the always-compiled part, cross-language receipt vectors in `fixtures/data-subject/v1/`, release wiring | D0 |
| D2 | `server` feature: migration, store, router, discovery, auth, erasure with the `baukit-erasure` fence, job handler, purge | D1 |
| D3 | Export operations and parts | D2 |
| T1 | `baukit-test` controlled target server for client tests | D1 |
| T2 | `baukit-test` service conformance check, exercised against an example product inside the crate's tests | D3, T1 |
| R | Gate, merge to `main`, release train | all |

Release wiring for a new crate touches `rust/Cargo.toml` (member and exact-version workspace dependency), `rust/release-plz.toml`, `scripts/publish-crates.sh` order, `docs/releasing.md` order list, the crate `CHANGELOG.md` and `README.md`, the root `README.md` crate table, the `CLAUDE.md` layout list and the crate count in `docs/platform/baukit-conventions.md`. `scripts/check-version-coherence.py` is the arbiter.

## Execution tracker

| Task | Status | Commit |
| --- | --- | --- |
| D0 | done | 26126192 |
| D1 | done | eea53d8a |
| D2 | in progress | |
| D3 | todo | |
| T1 | done | 59fd4b66 |
| T2 | todo | |
| R | todo | |

### Orchestration log

- 2026-10-20: plan written. Baukit main CI fails `cargo deny` on RUSTSEC-2026-0381 to 0383 since the jingle commit, so D0 goes first.
- 2026-10-20: D0 bumped webpki-roots and webpki-root-certs to 1.0.10 and rustls-webpki to 0.103.16; `cargo deny` passes and 822 workspace tests pass.
- 2026-10-20: D1 merged on the first pass. The contract document, the core crate and the vectors match the plan; OpenSSL verifies the positive vectors independently. An identifier type with invalid syntax fails during JSON parsing, so D2 must answer 422 `data_subject_request_invalid` for any body that fails to deserialize. D2 and T1 started in parallel.
- 2026-10-21: T1 merged after one fix by Claude. CI runs `cargo test` without features, so the target's tests behind `baukit-test/data-subject` never ran; `baukit-data-subject` now enables the feature as a dev-dependency. The workspace dependency on `baukit-data-subject` disables default features, so D2 must enable `server` explicitly.
