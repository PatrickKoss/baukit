# Changelog

All notable changes to `baukit-egress` are documented here.

## [Unreleased]

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

### Added

- Add `GuardedClient::execute_stream` and `EgressStreamResponse` with `status`,
  `headers` and chunked reads. Both response modes share the guarded send path,
  request tracing, timeouts and byte limits.
- Add `AllowedEndpoint`, `AllowedEndpointError` and `AddressPolicy::OnlyEndpoint`
  for an exact scheme, host and effective port. The HTTP flag selects HTTP only
  when true and HTTPS only when false.
- Add `AddressPolicy::PublicHttp` and `EgressOptions::with_redirect_responses`.
  Opted-in redirects return status and headers with an empty body and are never
  followed. Defaults still require public HTTPS and reject redirects.

### Fixed

- Block metadata addresses `100.100.100.200` and `fd20:ce::254` under every
  policy, including IPv4-mapped and well-known NAT64 forms where applicable.
- Give timeout tests time to receive headers under concurrent workspace load,
  while keeping a short bound on reads after the streaming deadline expires.

### Breaking

- `AddressPolicy` gains `PublicHttp` and `OnlyEndpoint`.
  `DestinationRejection` gains `Endpoint`. Both enums are now
  `#[non_exhaustive]`; downstream matches need a fallback arm. This requires
  a minor release during 0.x.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

### Added

- `PrivateNetwork` constructs and parses CIDR networks inside private or shared
  address space. `PrivateNetworkError` reports invalid notation, prefixes,
  host bits, ranges, mapped IPv6 input, and empty lists without echoing input.
- `PrivateNetworks` holds a non-empty network list. `AddressPolicy::AllowPrivate`
  permits HTTPS to public and listed addresses and plain HTTP to listed addresses
  only. Every DNS answer must pass. Cloud metadata and private NAT64 answers stay
  blocked. IPv4-mapped answers match listed IPv4 networks and connect as IPv4.

### Changed

- Breaking: `AddressPolicy` and `EgressOptions` implement `Clone`, but no longer
  `Copy`. Policy methods borrow `self`. `EgressOptions::policy()` returns
  `&AddressPolicy`. `validate_destination` and `resolve_destination` take
  `&AddressPolicy`.
- Breaking: `EgressOptions::with_policy`, `with_max_response_bytes`,
  `with_error_body_limit`, and `with_max_retry_after` are no longer `const`.

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

### Fixed

- Failed or timed-out error body reads preserve the HTTP status and retry class,
  including the capped `Retry-After` delay, and return no captured body.

### Added

- `EgressOptions::with_error_body_limit(bytes)` captures bounded non-`2xx` bodies.
  It is off by default. The limit must be nonzero and no larger than `max_response_bytes`.
  Reads use the existing request timeout and stop at the limit without a size error.
- `ErrorBody` exposes `bytes()`, `truncated()`, `content_type()`, and `problem_code()`.
  The code comes from the shared Baukit error envelope. `EgressError::error_body()`
  returns the captured body. `Display` omits bodies; `Debug` shows only their length,
  truncation flag, and problem code. Failure telemetry logs the code without raw bytes.

### Changed

- Breaking: `EgressError::Status` adds `body: Option<ErrorBody>`. Add `..` to matches
  that only need `status` and `class`.
- Every `5xx` with a valid `Retry-After` now carries `RetryAfter`, capped by
  `max_retry_after`. Without a usable header, `504` stays `Timeout` and other `5xx`
  stay `Unavailable`.

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

### Added

- `EgressOptions::with_forbidden_rate_limit` opts in to retryable `403` rate limits,
  and `forbidden_rate_limit` reports whether it is enabled. `GuardedClient` recognizes
  usable retry headers or `x-ratelimit-remaining: 0` with an optional reset epoch.
  Delays respect `max_retry_after`. Other `403` responses and all `401` responses stay
  revoked. The option is off by default.

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

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
