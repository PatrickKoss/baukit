# Changelog

All notable changes to `baukit-egress` are documented here.

## [Unreleased]

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
