# ADR 0025: Erasure commits in one transaction behind a subject fence, and data subject requests return signed receipts

## Status

Accepted, 2026-11-27.

## Context

A user who deletes their product profile expects the rows gone and the old
session dead. The product database and the identity provider are two systems,
so a crash or a provider outage can leave one changed and the other not. A
product write that started before the erase must not recreate the profile
afterwards. Some products share one provider account across several apps, so
deleting that account is not always wanted.

Privacy tools also ask services to erase or export a person found by
identifiers. They need a wire format that services outside baukit can
implement and a receipt they can verify without trusting the transport.

## Decision

`baukit-erasure` owns profile erasure. `ErasureService::new` takes an explicit
`IdentityRetention`. There is no default.

- `ErasureService::erase` calls the product's `ProductErasure::erase` on one
  transaction, then writes the keyed receipt and the subject fence in that same
  transaction. The commit is the acceptance boundary.
- `IdentityRetention::Retain` completes the receipt in that transaction. It
  enqueues no job and calls no provider. The provider account and its sessions
  stay, and the fence keeps old sessions out of this product.
- `IdentityRetention::Delete { deleter, provider_id, inline_timeout,
  max_attempts }` also enqueues an `identity.account.delete` job on
  `baukit-jobs`. After commit it tries the provider once within
  `inline_timeout`. The worker in `worker.rs` retries up to `max_attempts`.
  Failed operations wait for operator repair. A provider 404 counts as deleted
  only with the provider's "user not found" body.
- The fence stores `erased_at`. Product writes call `guard_subject` inside
  their own transaction, under the subject advisory lock. It admits a session
  only with a login after the fence and no unfinished operation. A later login
  with the same `sub` starts a fresh, empty profile.
- Products copy the reference SQL `POSTGRES_MIGRATION_SQL` and
  `POSTGRES_REONBOARDING_MIGRATION_SQL` after the `baukit-jobs` migrations, as
  [ADR 0026](0026-reference-sql-migrations.md) describes.

`baukit-data-subject` implements the v1
[data subject contract](../platform/data-subject-contract.md). Its core has no
HTTP or database dependency: wire types, `normalize_request`, and Ed25519 DSSE
receipts through `ReceiptSigner` and `ReceiptVerifier`. The default `server`
feature adds discovery, authenticated erasure routes, a PostgreSQL operation
store and a durable worker. Export is off until the product calls
`DataSubjectConfig::with_export`.

The client sequence and conformance helpers follow the
[product-profile erasure contract](../platform/product-profile-erasure-contract.md):
`eraseProductProfile` in `@baukit/data-contracts`, and
`check_product_profile_erasure_conformance` in `baukit-test`.

## Enforced by

- The `models` gate step runs `models/check.sh` on
  [`models/ErasureSaga`](../../models/ErasureSaga/README.md). TLC must find no
  violation of `CompletedMeansErased` and `NoStaleAdmission` in the fixed
  design.
- `rust/crates/baukit-erasure/tests/postgres.rs`:
  `atomic_rollback_after_product_callback`,
  `retained_identity_completes_replays_and_fences_without_a_job`,
  `retained_identity_completion_failure_rolls_back_product_receipt_and_fence`,
  `worker_retries_completes_and_removes_subject_payload`,
  `all_fence_checks_require_a_later_login_and_every_operation_completed`.
- `rust/crates/baukit-erasure/tests/fenced_writes.rs`:
  `write_admitted_before_an_erase_that_commits_first_is_rejected` and
  `erase_waits_for_a_guarded_write_and_then_deletes_its_rows`.
- `rust/crates/baukit-erasure/tests/keycloak_http.rs`:
  `unexplained_not_found_and_failed_probe_never_count_as_deleted`.
- `rust/crates/baukit-data-subject/tests/receipts.rs`:
  `runtime_vectors_match_labels_and_signer_reproduces_exact_envelope_bytes`
  checks the signer against the OpenSSL vectors in `fixtures/data-subject/v1/`.
- These run in the `rust-tests` gate step with `--include-ignored`.

## Consequences

Products get erasure that is atomic for their own rows, durable for the
provider account, and safe against writes that race the erase. They give up
plain writes on subject data: every write that can recreate a profile has to go
through `guard_subject`. Delete mode holds a job row lock and a pool connection
during the inline attempt, so `inline_timeout` has to stay short. Losing the
hash key breaks receipt authorization and fences.

## History

- 2026-10-03, `05386531`: durable identity account deletion added. Erasure
  always deleted the provider account.
- 2026-10-09, `9ebcd858`: `IdentityRetention` became required and added
  `Retain`, for apps that share one provider account.
- 2026-10-20, `eea53d8a`: the data subject v1 contract and receipt core added.
- This ADR replaces the erasure bullet in
  [ADR 0001](0001-product-experience-package-boundaries.md).
