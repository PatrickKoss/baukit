# ADR 0024: Suite apps link with PKCE, deliver HMAC-signed events and verify Ed25519 entitlement tickets

## Status
Accepted, 2026-11-27.

## Context
Apps in one suite exchange events for the same user, and the hub tells products what each account may use. Each peer needs proof that a request came from the linked app, that a replay flag was not flipped, and that an entitlement came from the hub.

Shared secrets fit peer links, because each link has its own. Entitlements are read by many products, so they need a key only the hub can sign with.

## Decision
`baukit-suite` links two apps with an OAuth-style code flow and PKCE S256 in `domain/link_protocol.rs`. Each link gets its own secret, sealed with the credential vault per [ADR 0018](0018-credential-vault-records.md).

Events go out through `baukit-jobs`. Each delivery carries `x-suite-signature`, HMAC-SHA256 from `baukit_core::webhook_signature::sign_webhook_hmac_sha256`, and `x-suite-signature-v2` from `domain::sign_suite_v2`, which also binds the `x-suite-replay` flag. `services/ingest.rs` `signatures_valid` requires v1 and, when present, v2. The platform receiver verifies hub notifications with `verify_webhook_hmac_sha256`.

Entitlement tickets are Ed25519 JWTs. `platform::tickets::verify_ticket` checks them against a `TicketJwks`, and `TicketVerifier` with `GuardedJwksSource` fetches keys through `baukit-egress`.

The wire contracts are in [suite events](../platform/suite-events.md) and the [platform receiver](../platform/platform-receiver.md).

## Enforced by
- `a_v2_signature_binds_the_replay_flag`, `an_unsigned_replay_flag_is_refused_not_downgraded` and `a_v2_signature_does_not_verify_as_v1_or_for_another_body` in `rust/crates/baukit-suite/src/services/ingest.rs`.
- `receiving_signature_fixture_matrix_uses_raw_bytes` in `rust/crates/baukit-suite/tests/protocol.rs` and `platform_signature_vectors_preserve_raw_utf8_and_empty_bodies` in `tests/platform_signatures.rs`.
- `shared_ticket_vectors` and `ticket_metadata_none_algorithm_and_large_revisions_are_rejected` in `rust/crates/baukit-suite/tests/platform.rs`, over `fixtures/entitlement-tickets`. Gate step `rust-tests`.
- TLA+ models [M-04 SuiteLink](../../models/SuiteLink/README.md) and [M-05 SuiteInbox](../../models/SuiteInbox/README.md). Gate step `models`.

## Consequences
Products get authenticated peer events and offline-verifiable entitlements. They must keep link secrets sealed and the hub's JWKS URL configured. A v1-only receiver still accepts events but cannot detect a flipped replay flag.

## History
- 2026-10-22, `68d0db3d`: platform receiver and Ed25519 ticket verifier added.
- 2026-11-25, `97372288`: v2 signature added to bind the replay flag, which v1 left unsigned.
