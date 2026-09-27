# Guarded egress evidence

Item 16 of the [cross-product feature plan](../cross-product-feature-plan.md), first deliverable.
The webhook study, the second deliverable, has its own note:
[52-webhook-delivery-study.md](52-webhook-delivery-study.md).

## Source revisions

Product revisions come from each repository's `main` ref on 2026-09-28.

- Tiefgang `2d37a06`: `backend/crates/tiefgang-domain/src/webhooks.rs:79-95` (`public_webhook_ip`)
  and `97-114` (`public_ipv4`), `backend/crates/tiefgang-services/src/webhooks.rs:150-184`
  (`validate_url`), `backend/crates/tiefgang-worker/src/lib.rs:122-159` (`delivery_client`) and
  `161-171` (`valid_resolved_addresses`).
- Runtime Analyzer `d47bfd5`: `finops-integrations/src/lib.rs:22-69` (`client_for`), `71-107`
  (`public_address`, `public_v4`), and `109-141` (its own response classifier).
- Solo Leveling System `3461eaf`: `sl-notifications/src/http.rs:23` (`reqwest::Client::default()`),
  `sl-bin/src/bin/worker.rs:62-89`.
- Eigenruhe `f74cebb` (the plan cites `e44ff88`; the lines it names are unchanged):
  `eigenruhe-integrations/src/hub.rs:80-160`.
- IANA registries, fetched on 2026-09-28:
  - [IPv4 Special-Purpose Address Registry](https://www.iana.org/assignments/iana-ipv4-special-registry/),
    last updated 2025-10-09.
  - [IPv6 Special-Purpose Address Registry](https://www.iana.org/assignments/iana-ipv6-special-registry/),
    last updated 2025-10-09.
  - [IPv6 Global Unicast Address Assignments](https://www.iana.org/assignments/ipv6-unicast-address-assignments/),
    last updated 2025-10-10.
  - [IPv6 Address Space](https://www.iana.org/assignments/ipv6-address-space/), last updated
    2025-10-23.
- `reqwest` 0.12.28 as pinned in the workspace (`rustls-tls`, no HTTP/2), `url` 2.5.8.

## Baukit owner

New crate `baukit-egress` (`rust/crates/baukit-egress`). Shared vectors in
`fixtures/egress/address-policy-v1.json`.

## Step 1: one address policy

### How the two filters compare

Both products check the scheme and user info, resolve the host, check every answer, pin the
connection to the checked answers, and turn redirects off. They disagree on these points:

| Case | Tiefgang | Runtime Analyzer | Baukit |
|---|---|---|---|
| `3fff::/16` outside `3fff::/20` | allowed | blocked | allowed |
| `3ffe::/16` (former 6bone, reserved) | allowed | allowed | allowed |
| `192.88.99.0/24` (6to4 relay anycast) | blocked | allowed | blocked |
| `64:ff9b::/96` carrying a public IPv4 | blocked | blocked | allowed |
| `64:ff9b::/96` carrying a private IPv4 | blocked | blocked | blocked |
| IPv4-mapped `::ffff:a.b.c.d` | judged by the IPv4 | judged by the IPv4 | judged by the IPv4 |
| DNS lookup timeout | bounded | none | bounded, 3 s default |
| Proxy settings from the environment | ignored | ignored | ignored |

### The `3fff::/16` disagreement

The IPv6 Special-Purpose registry lists `3fff::/20` as Documentation [RFC9637], "Globally
Reachable: False". The IPv6 Global Unicast Address Assignments registry lists the same `3fff::/20`
as Documentation, assigned 2024-07-23, marks the blocks before it up to `3ffe::/16` as reserved by
IANA, and has no entry for the rest of `3fff::/16`. So the registry basis for blocking stops at
`3fff::/20`.

Tiefgang matches the registry: it blocks `3fff:0000` through `3fff:0fff` and allows
`3fff:1000::` and above. Runtime Analyzer allows only a first segment in `0x2000..=0x3ffe`. That
blocks all of `3fff::/16`, which goes further than the registry, and still allows the reserved
`3ffe::/16`, so its rule is not "block unallocated space" either.

Baukit blocks `3fff::/20` and nothing else in `3fff::/16`. The policy follows the special-purpose
registries and does not track allocation status. Unallocated space inside `2000::/3` does not route
today, so treating it as public opens no path, and an allocation table would need a release every
time IANA hands out a block. The vectors pin both edges: `3fff:fff:ffff::1` is blocked,
`3fff:1000::1` and `3fff:ffff::1` are allowed, and `3ffe::1` is allowed.

### Other decisions and their registry entries

- `192.88.99.0/24` is "Deprecated (6to4 Relay Anycast)" [RFC7526] in the IPv4 registry, and
  `192.88.99.2/32` (6a44 relay) is not globally reachable. Baukit blocks the whole /24, as
  Tiefgang does.
- `192.0.0.0/24` is blocked whole. The registry marks `192.0.0.9/32` (PCP anycast) and
  `192.0.0.10/32` (TURN anycast) globally reachable, but neither is a webhook or provider
  destination.
- `2001::/23` (IETF Protocol Assignments) is blocked whole, including its globally reachable
  exceptions `2001:1::1-3`, `2001:3::/32` (AMT), `2001:4:112::/48` (AS112), `2001:20::/28`
  (ORCHIDv2), and `2001:30::/28` (drone identifiers). None of them serves HTTP to a sender. AS112
  outside the block (`192.31.196.0/24`, `2620:4f:8000::/48`) stays public, as the registry says.
- `2002::/16` (6to4, reachability "N/A") is blocked because the embedded IPv4 can be private.
- `64:ff9b::/96` is globally reachable per [RFC6052]. Both products blocked it, which breaks every
  destination on an IPv6-only cluster with DNS64. Baukit judges the embedded IPv4 instead, so
  `64:ff9b::808:808` is allowed and `64:ff9b::a9fe:a9fe` (metadata) is blocked.
  `64:ff9b:1::/48` [RFC8215] is not globally reachable and stays blocked.
- Everything outside `2000::/3` (IPv6 Address Space registry) is blocked, which covers loopback,
  unspecified, IPv4-compatible `::/96`, discard `100::/64`, the `100:0:0:1::/64` dummy prefix
  [RFC9780], `5f00::/16` SRv6 SIDs [RFC9602], unique-local `fc00::/7`, link-local `fe80::/10`,
  site-local `fec0::/10`, and multicast `ff00::/8`.
- IPv4 multicast `224.0.0.0/4` comes from the IPv4 address space registry, not the special-purpose
  one. It is blocked.

A network-specific NAT64 prefix (RFC 6052 section 2.2) is not recognized. Its synthesized
addresses look like ordinary global unicast, so a translator on such a prefix can reach private
IPv4 space. Deployments with one must resolve IPv4 answers or put an egress network policy in
front. The README says so.

### Published vectors

`fixtures/egress/address-policy-v1.json`, version 1, names its four registry sources and holds:

- 65 address cases with the registry entry each one comes from and the expected result under both
  policies. They cover private, loopback, link-local, unique-local, documentation, IPv4-mapped,
  NAT64 well-known and local-use, 6to4, Teredo, benchmarking, multicast, reserved, and the
  `3fff` edges.
- 18 destination cases: schemes, user info, fragments, and address literals written as dotted,
  decimal (`2130706433`), hex (`0x7f.1`), mapped, NAT64, and IPv6 forms. WHATWG URL parsing turns
  decimal and hex forms into `127.0.0.1` before the check.
- 14 resolution cases, each a sequence of lookups: dual stack, a single private answer, mixed
  answers with the bad one first, last, or third of three, the empty answer, and
  `answer-changes-between-validation-and-connect`, where the first lookup returns
  `93.184.215.14` and the second `10.0.0.1`.

A mutation check widened the `3fff` block to `/16`; the address vectors failed, then passed again
after the revert.

## Step 2: the guarded client

### Crate or feature

A new crate. `baukit-http` is server middleware and has no HTTP client dependency. Adding the
client there would pull `reqwest`, `rustls`, and `url` into every product that only wants the error
envelope, or it would hide behind a feature. CI runs `cargo test` and `cargo clippy` without
`--all-features`, so a feature would ship untested. `baukit-egress` depends on `baukit-http` for
`classify_http_status` and on workspace dependencies only. `Cargo.lock` gained the
`baukit-egress` entry and no third-party package.

### How it works

`GuardedClient::with_resolver` builds one `reqwest::Client` with:

- a `reqwest::dns::Resolve` adapter around the `Resolver` port. It runs the lookup under the
  resolve timeout, rejects the whole answer when any address fails the policy, and hands reqwest
  only the checked addresses. reqwest calls it once per new connection, so a keep-alive reuse
  stays on a checked address and every new connection re-checks. TLS SNI and `Host` keep the
  original name.
- `redirect::Policy::none()`, `no_proxy()`, `referer(false)`, `https_only` unless the policy is
  `AllowLoopback`, `connect_timeout`, and `timeout` (the whole request, body included).

`execute` validates the URL first. hyper skips the resolver for IP-literal hosts, so
`validate_destination` checks literals itself. A non-`2xx` status becomes
`EgressError::Status` with `classify_http_status(status, headers, &[])`, and its body is never
read. A `2xx` body is read chunk by chunk and stops at `max_response_bytes`, with or without
`Content-Length`.

### Tests

`rust/crates/baukit-egress/tests/guarded_client.rs` runs against raw Tokio listeners that count
connections, with a counting resolver or `StaticResolver`:

- a POST reaches the resolved loopback server under `AllowLoopback`;
- `PublicOnly` blocks a loopback answer, and the server sees zero connections;
- mixed answers in either order are blocked with zero connections;
- a changed DNS answer: the first request connects, the second lookup returns
  `169.254.169.254` and is blocked, the resolver ran twice, and the server saw one connection;
- a `302` to a second local server and a `302` to `169.254.169.254` both come back as
  `Status { 302, Permanent }`, and the second server sees zero connections;
- loopback literals in dotted, mapped, and decimal form are blocked without a connection;
- `http`, user info, and fragments are refused with the matching `DestinationRejection`;
- `429` with `Retry-After: 7`, `503`, `504`, `401`, and `404` map to `RetryAfter(7s)`,
  `Unavailable`, `Timeout`, `Revoked`, and `Permanent`;
- oversized bodies with `Content-Length` and chunked are refused; bodies at the limit pass;
- a silent server hits the request timeout, a pending resolver hits the resolve timeout;
- an unknown name is `Resolve` and a closed port is `Transport`, both retryable.

`tests/telemetry_redaction.rs` captures every span and event at `TRACE`, sends requests whose path,
query, user info, and `Authorization` header hold marker secrets to a `500` server, a closed port,
and a refused URL, and asserts that no marker appears in the error's `Display` or `Debug`, the
request's `Debug`, or the captured log. It runs as its own test binary: with other tests on
parallel threads, tracing's callsite interest cache sometimes dropped the span, which made the
check flaky rather than wrong. A mutation check that let reqwest follow three redirects failed the
redirect test.

## Public types

From `baukit-egress`:

- `GuardedClient` (`new`, `with_resolver`, `options`, `execute`), `EgressRequest` (`new`, `get`,
  `post`, `with_headers`, `with_body`, getters), `EgressResponse` (`status`, `headers`, `body`,
  `into_body`), `EgressClientError`, `validate_destination`.
- `AddressPolicy` (`PublicOnly`, `AllowLoopback`; `permits`, `permits_all`, `allows_plain_http`),
  `is_public_address`.
- `Resolver`, `ResolveFuture`, `ResolveError`, `SystemResolver`, `StaticResolver`,
  `resolve_destination`.
- `EgressError` (`Destination`, `BlockedAddress`, `Resolve`, `Timeout`, `Transport`, `Status`,
  `ResponseTooLarge`; `code`, `retry_class`, `is_retryable`), `DestinationRejection` (`Scheme`,
  `Credentials`, `Fragment`, `MissingHost`; `code`).
- `EgressOptions` (`with_policy`, `with_resolve_timeout`, `with_connect_timeout`,
  `with_request_timeout`, `with_max_response_bytes`, getters) and `EgressOptionsError`.

## Supported runtimes

Any Tokio runtime, current-thread or multi-thread. `SystemResolver` uses
`tokio::net::lookup_host`, which runs `getaddrinfo` on the blocking pool. HTTP/1.1 over rustls,
because the workspace `reqwest` pin has no HTTP/2 feature.

## Failure behavior

| Failure | `EgressError` | `RetryClass` |
|---|---|---|
| Wrong scheme, user info, fragment, no host | `Destination(_)` | `Permanent` |
| Literal or any resolved answer outside the policy | `BlockedAddress` | `Permanent` |
| Lookup error or empty answer | `Resolve` | `Unavailable` |
| Lookup, connect, or request timeout | `Timeout` | `Timeout` |
| Connection refused or reset, TLS failure | `Transport` | `Unavailable` |
| Non-`2xx` status, redirects included | `Status { status, class }` | `classify_http_status` |
| Body over the limit | `ResponseTooLarge { limit }` | `Permanent` |

A timeout after the request was written is ambiguous: the receiver may have acted on it. The
error is still `Timeout`, and callers that retry must reuse the same idempotency or delivery key.
`RetryAfter` carries the receiver's value uncapped; the caller caps it.

## Privacy boundary

No `EgressError` variant carries a URL. `Transport` wraps the reqwest error after `without_url()`.
The `egress.request` span records the method, host, port, status, and `error.type`; the failure
event adds the error code and its `Display`. `EgressRequest`'s `Debug` shows the method, host,
header names, and body length. The resolver logs the host at `debug` on a failed lookup. Paths,
queries, user info, header values, and bodies never reach errors or telemetry, and the telemetry
test pins that.

## Breaks

None. `baukit-egress` is new, and the vectors are a new fixture.

## Product code a later adoption removes

- Tiefgang: `public_webhook_ip` and `public_ipv4` in `tiefgang-domain/src/webhooks.rs:79-114`;
  `delivery_client` and `valid_resolved_addresses` in `tiefgang-worker/src/lib.rs:122-171`; the
  scheme, user-info, and address checks in `validate_url` (`tiefgang-services/src/webhooks.rs:150-184`)
  become `validate_destination`, keeping its length limits.
- Runtime Analyzer: `client_for`, `public_address`, and `public_v4` in
  `finops-integrations/src/lib.rs:22-107`. Its classifier at `109-141` becomes
  `EgressError::retry_class` plus a local `Retry-After` clamp.
- Solo Leveling System: the `reqwest::Client::default()` at `sl-notifications/src/http.rs:23` and
  its wrapper in `sl-bin/src/bin/worker.rs:62-89` become a `GuardedClient`. This closes the open
  SSRF gap for webhooks and the user-supplied Slack and Discord URLs.
- Eigenruhe: optional. It posts to one operator-configured hub URL, so the risk is lower, but the
  hub client still follows redirects.

## Open decisions

- The integration reliability recipe retries `425`; `classify_http_status` calls it `Permanent`.
  The egress client follows the classifier. Pick one before a delivery runtime exists.
- Whether `baukit-http` should offer a capped `Retry-After`. Runtime Analyzer clamps to 1 to 300
  seconds; Tiefgang and the classifier do not.
- A network-specific NAT64 prefix option on `AddressPolicy`, if a deployment needs one.
