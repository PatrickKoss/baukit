# Changelog

## [Unreleased]

## [0.10.0] - 2026-10-08

## [0.9.0] - 2026-10-07

- Add Clerk and WorkOS user deletion adapters with typed failures and durable worker retries.

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

- Test generated backend client recreation and reconciliation against Keycloak before account deletion.

- Use aws-lc-rs for cryptographic operations instead of a second ring dependency. Keep the supported algorithms and wire formats.

- Clarify that wire receipts always have an operation ID. Test missing and null IDs against the shared receipt vectors.

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

- Return failed erasure replays as terminal 200 receipts. Keep the operation ID and failed job for repair. Share wire receipt tests with the TypeScript client.

## [0.7.0] - 2026-10-04

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

- Log inline failure classes without subjects and test the token acquisition timeout.

- Add transactional product erasure, keyed subject fences, durable identity deletion, and a Keycloak admin adapter.
