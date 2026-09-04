# Typed API token store errors

## Source product files

- `/home/patrick/projects/leitbild/backend/crates/leitbild-postgres/src/api_token.rs`
- `/home/patrick/projects/leitbild/backend/crates/leitbild-bin/src/lib.rs`
- `/home/patrick/projects/leitbild/backend/crates/leitbild-api/src/api_token.rs`
- `/home/patrick/projects/leitbild/backend/tests/postgres_integration.rs`

## Observed repeated glue

Leitbild's PostgreSQL and memory adapters encode the active-token limit as
`limit_exceeded:api_tokens_active:N`. Its API layer parses that string to recover the limit. SQLx
errors use the same `String` channel, so policy data and private database text have no type boundary.

## Baukit owner

`baukit-auth` owns the store error contract and service mapping. `baukit-test` owns the scriptable
memory adapter.

## Public types and errors

- `ApiTokenStoreError::{Internal, PolicyRejected}`
- `ApiTokenPolicyRejection` with a bounded snake_case code and up to eight named `u32` details
- `ApiTokenPolicyRejectionError`
- `ApiTokenError::{PolicyRejected, Storage}`

## Product-owned inputs

Products choose policy codes, numeric limits, database schemas, SQL, retention rules, and HTTP
status or envelope mappings.

## Required cases

- Concurrency: the product adapter must enforce its token limit in the same transaction or lock as
  token creation.
- Failure: internal adapter errors become `ApiTokenError::Storage`; policy failures retain their
  code and numeric details.
- Privacy: internal text never appears in `ApiTokenError` display text. Authentication still maps
  malformed, unknown, hash-mismatched, and revoked credentials to one public result.
- Cleanup: revoked-token retention remains an adapter concern and is unchanged by this contract.

## Supported runtimes

Rust 1.95 or newer on the server runtimes supported by the Baukit Rust workspace. The store contract
does not require a specific database.

## Product adoption change

A Leitbild adoption pull request must update its PostgreSQL and memory adapters, replace the encoded
limit string with `ApiTokenPolicyRejection`, remove `active_limit_maximum`, and update the related
tests. No adoption pull request is open yet.
