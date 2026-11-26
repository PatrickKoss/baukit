# ADR 0018: Stored secrets are sealed as credential vault v2 records

## Status
Accepted, 2026-11-27.

## Context
Services keep OAuth tokens, provider keys, browser sessions and suite link secrets in PostgreSQL. Anyone with write access to the table could paste a ciphertext from one row into another, or drop a field, and the first format had no way to notice.

Field-by-field sealing binds a ciphertext to its scope, key version and field name, but it cannot tell whether a record opened whole or an older field was swapped in.

## Decision
`baukit-credential-vault` seals secrets with AES-256-GCM from a versioned keyring. The highest version writes and every listed version reads. `CredentialCipher::seal_record` and `open_record` bind each field to the scope, key version, a random per-seal `record_id` and the sorted list of all field names, under the `baukit-credential-vault/record/v2` label. Dropping, adding or renaming a field, or mixing fields from two seals, fails as `CredentialVaultError::DecryptionFailed`. `encrypt` and `decrypt` remain for v1 rows. See the [crate README](../../rust/crates/baukit-credential-vault/README.md).

`baukit-auth` browser sessions and `baukit-suite` link secrets and PKCE verifiers write v2 records and store `record_id` in a nullable column. A NULL `record_id` opens through the v1 path. `SuiteDeliveryService::reseal_legacy_secrets` re-seals v1 suite secrets in batches during the hourly cleanup. Old browser sessions expire instead.

## Enforced by
- `dropping_any_field_fails`, `substituting_a_field_from_an_earlier_seal_fails` and `truncating_below_the_tag_fails` in `rust/crates/baukit-credential-vault/src/record.rs`.
- `another_scope_fails`, `renamed_field_fails`, `another_key_version_fails` and `single_byte_change_fails` in `rust/crates/baukit-credential-vault/tests/properties.rs`.
- `v2_rows_reject_tampering_and_v1_rows_written_before_the_migration_still_open` in `rust/crates/baukit-auth/src/browser_session/record_tests.rs`, and `v2_secrets_reject_tampering_and_v1_rows_written_before_the_migration_still_open` and `reseal_converts_v1_secrets_once_skips_unreadable_ones_and_leaves_v2_alone` in `rust/crates/baukit-suite/src/record_tests.rs`. These need Docker and run with `--include-ignored`. Gate step `rust-tests`.
- Nothing stops product code from calling `encrypt` for new rows.

## Consequences
Products get tamper detection per record and key rotation with no data migration. They store one more column per sealed record. Swapping in a whole older record is a rollback only the storage layer can refuse.

## History
- 2026-11-25, `880ea9b4`: `seal_record` and `open_record` added. Before, v1 sealed every field on its own.
- 2026-11-26, `96187ba4`: suite cleanup re-seals v1 link and request secrets.
