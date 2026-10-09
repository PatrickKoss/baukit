# Changelog

All notable changes to `baukit-egress` are documented here.

## [Unreleased]

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

## [0.9.0] - 2026-10-07

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

## [0.6.0] - 2026-10-02

### Changed

- Moved to reqwest 0.13 with its `rustls` backend. HTTPS clients now verify servers against the
  operating system's trust store through rustls-platform-verifier instead of the bundled
  webpki roots, so a container image needs CA certificates (distroless `cc` ships them). The
  TLS crypto provider is aws-lc-rs instead of ring.

## [0.5.2] - 2026-09-30

### Added

- `ResponseBody` and `EgressRequest::with_response_body`. `ResponseBody::Discard`
  drops a `2xx` body unread, so a webhook receiver that answers `2xx` with a body
  above `max_response_bytes` counts as delivered instead of failing with the
  permanent `ResponseTooLarge`. `ResponseBody::Read` stays the default.
- `AddressPolicy::permits_plain_http(address)`.
- `fixtures/egress/address-policy-v1.json` gains five plain-http literal cases.

### Breaking

- Under `AddressPolicy::AllowLoopback`, plain `http` reaches loopback
  addresses only. An `http` URL with a non-loopback literal, or a host name
  with any non-loopback answer, fails with
  `EgressError::Destination(DestinationRejection::Scheme)` before a
  connection. Before, the policy turned off https-only for every host. Local
  development setups that sent `http` to a non-loopback host must use `https`
  or a loopback address.
- `EgressRequest`'s `Debug` output adds the `response_body` field.

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Added

- First release of `baukit-egress`, a guarded outbound HTTP client for
  destinations that users supply.
- `GuardedClient` resolves through the `Resolver` port, rejects the lookup when
  any answer fails the `AddressPolicy`, connects only to the checked answers,
  never follows redirects, ignores proxy settings, and bounds the lookup, the
  connect, the request, and the response body. `EgressRequest` and
  `EgressResponse` carry the request and the read body.
- `AddressPolicy` with `PublicOnly` and `AllowLoopback`, and
  `is_public_address`, following the IANA special-purpose registries. IPv4-mapped
  addresses and `64:ff9b::/96` are judged by their embedded IPv4 address.
- `Resolver`, `ResolveFuture`, `ResolveError`, `SystemResolver`,
  `StaticResolver`, and `resolve_destination`.
- `validate_destination` checks scheme, user info, fragment, host, and address
  literals without a lookup.
- `EgressError` with `code()`, `retry_class()`, and `is_retryable()`.
  Non-`2xx` statuses are classified by `baukit_http::classify_http_status`.
  `DestinationRejection` names each refused URL shape. No error carries the URL.
- `EgressOptions` with `EgressOptionsError`: 3 s lookup, 5 s connect, 10 s
  request, and 1 MiB body by default.
- `EgressOptions::with_max_retry_after` and `max_retry_after` cap the
  `Retry-After` delay in `EgressError::Status` at 300 s by default. A zero cap
  returns `EgressOptionsError::ZeroRetryAfterCap`.
- Shared vectors in `fixtures/egress/address-policy-v1.json`.

### Changed

- Break: `425 Too Early` is now `RetryClass::Unavailable`, following
  `baukit_http::classify_http_status`.
- Break: a `Retry-After` delay above the cap is clamped instead of reported
  as sent.
