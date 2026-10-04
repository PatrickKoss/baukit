# Changelog

## [Unreleased]

- Return failed erasure replays as terminal 200 receipts. Keep the operation ID and failed job for repair. Share wire receipt tests with the TypeScript client.

## [0.7.0] - 2026-10-04

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

- Log inline failure classes without subjects and test the token acquisition timeout.

- Add transactional product erasure, keyed subject fences, durable identity deletion, and a Keycloak admin adapter.
