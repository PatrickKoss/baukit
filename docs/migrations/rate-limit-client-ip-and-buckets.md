# Rate limiting: client IP, IPv6 buckets and amount releases

This release changes how `baukit-ratelimit` picks and names the IP bucket. Read
it before upgrading a product that runs behind a proxy.

## `trusted_proxy_hops` now defaults to 0

The old default of 1 trusted the rightmost `X-Forwarded-For` entry even when no
proxy was in front of the service, so any client could pick any bucket. The new
default ignores `X-Forwarded-For` and keys on the socket peer.

Behind an ingress that appends to `X-Forwarded-For`, set the count explicitly,
for example `<PREFIX>__RATE_LIMIT__TRUSTED_PROXY_HOPS=1` for one ingress hop.
Write down which proxies that number counts next to the setting. A value larger
than the real number of appending proxies lets clients choose their bucket.

Only the entry at `len - hops` is parsed. A garbage entry written by the client
to its left no longer moves the request into the proxy's bucket. An unparseable
trusted entry logs a warning and keys on the peer.

## The IP scope requires `ConnectInfo`

Requests without `ConnectInfo<SocketAddr>` used to share `rl:ip:unknown`. With
the IP scope enabled they now get a 500 and an error log. `baukit-runtime`
listeners provide `ConnectInfo`. Custom servers must use
`into_make_service_with_connect_info::<SocketAddr>()`, and tests must insert a
`ConnectInfo` extension.

## IPv6 keys bucket per /64

IPv6 clients share one bucket per network of `rate_limit.ipv6_prefix_len` bits
(default 64, valid 1 to 128). Keys change from `rl:ip:2001:db8::1` to
`rl:ip:2001:db8::/64`. IPv4-mapped IPv6 addresses use the IPv4 key. Old keys
expire on their own TTL, so each IPv6 client starts with a full bucket once.

## Redis token-bucket fields

The token-bucket script now stores integer `scaled_tokens` and `last_ms` hash
fields. The old `tokens` and `last_refill_ms` fields stay until the key's TTL
ends; tenant ACLs need no new commands. Existing
buckets start full once after the upgrade. No action is needed.

## Validation outside local

`BaukitConfig` validation now fails outside `local` when both the identity and
IP scopes are disabled. `fail_mode` still defaults to `open`, so a store outage
disables limiting. Set `closed` where abuse protection matters more than
availability.

## Amount releases name their window

`AmountBudget::release(subject, amount)` becomes
`release(subject, amount, window)`. Pass `decision.window` from the matching
`consume` (a new `Option<BudgetWindow>` field on `AmountBudgetDecision`). A
release after that window ended is a no-op instead of crediting the new window.

## Redis Sentinel retries

After a failed script call the Sentinel store re-resolves the master but only
retries when the connection was refused. A request that may have run is no
longer replayed, so a lost reply surfaces as a store error and follows
`fail_mode`.
