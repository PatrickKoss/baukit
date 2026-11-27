# ADR 0016: All outbound HTTP goes through baukit-egress with DNS pinning

## Status
Accepted, 2026-11-27.

## Context
Services send requests to URLs that users or tenants supply: webhook targets, OIDC issuers, provider endpoints. A plain `reqwest::Client` can be pointed at `169.254.169.254`, a private database, or a name whose DNS answer flips to a private address after the check.

Checking the URL once is not enough. The address that passed the check has to be the one the connection uses.

## Decision
`baukit-egress` is the only outbound HTTP client. `GuardedClient::execute` takes an `EgressRequest` and:

- checks address literals and every DNS answer against an `AddressPolicy`, and rejects the lookup if one answer is denied;
- connects only to the answers that passed, and runs a new lookup and check for each new connection;
- never follows redirects and ignores proxy environment variables;
- bounds lookup, connect, request time and body size. `EgressOptions::default()` uses 3 s, 5 s, 10 s and 1 MiB.

`AddressPolicy::PublicOnly` is the default. It allows globally reachable unicast addresses from the IANA special-purpose registries, judges IPv4-mapped and NAT64 `64:ff9b::/96` addresses by their IPv4 part, and denies the metadata addresses. `AllowPrivate` takes an explicit `PrivateNetworks` list. `is_public_address` implements the table. See the [crate README](../../rust/crates/baukit-egress/README.md).

`baukit-auth`, `baukit-erasure`, `baukit-mcp`, `baukit-push` and `baukit-suite` build their clients from it.

## Enforced by
- `lint/ast-grep/rules/rust-raw-http-client.yml`, severity `error`, flags `reqwest::Client::new()`, `Client::builder()`, `ClientBuilder::new()` and `reqwest::get` outside `baukit-egress`, `adapters/egress`, `baukit-test` and test code. `baukit check` fails on it. Gate step `baukit-check`.
- Kani K-03 in `rust/crates/baukit-egress/src/policy.rs`: `ipv4_public_matches_spec`, `ipv6_public_matches_spec` and `public_only_permits_matches_spec` prove that `is_public_address` and `AddressPolicy::permits` equal the spec for every address.
- Kani K-04 in the same file: `ipv4_mask_has_prefix_leading_ones` and `ipv6_mask_has_prefix_leading_ones` prove that `ipv4_mask` and `ipv6_mask` never panic and set exactly `prefix` leading ones. Gate step `kani`.
- `rust/crates/baukit-egress/tests/guarded_client.rs`: `a_changed_dns_answer_never_reaches_a_connection`, `one_disallowed_answer_blocks_the_whole_lookup`, `redirects_are_returned_and_never_followed` and `address_literals_are_checked_before_connecting`. Gate step `rust-tests`.
- `rust/crates/baukit-egress/tests/address_policy_vectors.rs` runs the shared vectors in `fixtures/egress`.

## Consequences
Products get SSRF protection, timeouts and size limits without writing them. They give up redirects, proxies and private destinations unless they opt in per policy. A network-specific NAT64 prefix is not recognized.

## History
- 2026-09-28, `19385d5d`: `baukit-egress` added with published address vectors.
- 2026-11-07, `e5908610`: the `rust-raw-http-client` rule made the client mandatory.
