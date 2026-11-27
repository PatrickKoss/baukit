# Control outbound HTTP and request rates

At the end of this page your backend sends every outbound request through
`baukit-egress`, the chart lets those requests leave the pod, and your routes sit
behind `baukit-ratelimit` with settings that fit each environment. The page is
for product developers who add a webhook sender, a provider client or a new
route group, and for whoever sets the deployment values.

## Send outbound requests through `GuardedClient`

Outbound HTTP goes through `baukit_egress::GuardedClient`. Do not build a raw
`reqwest::Client`. The guarded client checks the URL and every DNS answer
against an `AddressPolicy`, connects only to the answers it checked, never
follows redirects and ignores `HTTP_PROXY` and `HTTPS_PROXY`. Defaults are a 3 s
lookup, a 5 s connect, a 10 s request and a 1 MiB response body.

The smallest call, from `rust/crates/baukit-egress/README.md`:

```rust,ignore
use baukit_egress::{EgressOptions, EgressRequest, GuardedClient};

let client = GuardedClient::new(EgressOptions::default())?;
let response = client
    .execute(EgressRequest::post(url).with_headers(headers).with_body(body))
    .await?;
```

The crate is opt-in. The generated backend does not depend on it, and its
options are not part of `baukit_config::BaukitConfig`. Your product builds
`EgressOptions` in code, usually from its own config section.

## Pick the policy by who chose the destination

The question that decides the policy is who wrote the URL.

| Destination | Who supplies it | Policy |
|---|---|---|
| A public webhook URL or a provider API | a tenant or end user | `AddressPolicy::PublicOnly`, the default |
| One host a tenant configured, such as their own server | a tenant | `AddressPolicy::OnlyEndpoint(AllowedEndpoint::tenant(host, port, allow_http)?)` |
| One in-cluster service set in deployment config | the operator | `AddressPolicy::OnlyEndpoint(AllowedEndpoint::new(host, port, allow_http)?)` |
| Several in-cluster services, such as Prometheus | the operator | `AddressPolicy::AllowPrivate(PrivateNetworks::new([...])?)` |
| A public server that only speaks HTTP | either | `AddressPolicy::PublicHttp` |
| A test server on loopback | tests and local runs | `AddressPolicy::AllowLoopback` |

`AllowedEndpoint` has two constructors because the two origins need different
answers. `AllowedEndpoint::new` and `AllowedEndpoint::from_url` mark the
endpoint `EndpointOrigin::OperatorInternal`, so private, shared and loopback DNS
answers pass. `AllowedEndpoint::tenant` and `AllowedEndpoint::tenant_from_url`
mark it `EndpointOrigin::Tenant`, so only public answers pass. A tenant who
controls a DNS name therefore cannot point it at your database. Never pass a
tenant-supplied host to `AllowedEndpoint::new`. Both kinds match the scheme,
host and port exactly, and the `allow_http` flag selects HTTP only or HTTPS
only.

Every policy blocks the cloud metadata addresses `169.254.169.254`,
`100.100.100.200`, `fd00:ec2::254` and `fd20:ce::254`, also inside a listed
private network. `PrivateNetworks` accepts only ranges inside `10.0.0.0/8`,
`172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10` and `fc00::/7`. List the
smallest range that covers the service.

The in-cluster example from `rust/crates/baukit-egress/README.md`, compiled as a
doctest:

```rust,no_run
use baukit_egress::{AddressPolicy, EgressOptions, EgressRequest, GuardedClient, PrivateNetworks};
use url::Url;

# async fn collect() -> Result<(), Box<dyn std::error::Error>> {
let networks = PrivateNetworks::new(["10.43.0.0/16".parse()?])?;
let options = EgressOptions::default().with_policy(AddressPolicy::AllowPrivate(networks));
let client = GuardedClient::new(options)?;
let url = Url::parse("http://prometheus.monitoring.svc:9090/api/v1/query?query=up")?;
let response = client.execute(EgressRequest::get(url)).await?;
# let _ = response;
# Ok(())
# }
```

`AllowLoopback` exists for tests. Never select it outside the `local`
environment.

## Handle failures and retries

`EgressError::retry_class()` tells the caller what to do next, and
`EgressError::code()` gives a stable string for logs. A `429` or a `5xx` with
`Retry-After` becomes `RetryClass::RetryAfter`, clamped to
`EgressOptions::max_retry_after`, 300 s by default. `401` and `403` are
`Revoked` unless you enable `with_forbidden_rate_limit()` for APIs that report
rate limits with `403`. A webhook sender that ignores the receiver's answer sets
`ResponseBody::Discard`. The full table of variants and status classes is in the
crate README.

No error variant and no span field carries the URL path or query, because
webhook URLs often embed tokens there. Keep it that way when you log.

## Open the path in the chart NetworkPolicy

`GuardedClient` decides which addresses the code may reach. The cluster decides
which packets may leave the pod. Both have to agree, or the request fails with
`EgressError::Timeout` or `Transport`.

The `baukit-app` chart enables `networkPolicy.enabled` by default. Its policy
denies all egress except DNS to CoreDNS, the backing Postgres and Redis paths
the `backing` values select, and the rules you add. Add destinations at the
narrowest level that works:

| Value | Applies to |
|---|---|
| `networkPolicy.additionalEgress` | every process in the release |
| `api.networkPolicy.additionalEgress` | API pods only |
| `worker.networkPolicy.additionalEgress` | worker pods only |
| `migration.networkPolicy.additionalEgress` | migration Job pods |
| `seed.networkPolicy.additionalEgress` | seed Job pods |

Each value is a raw list of Kubernetes NetworkPolicy egress rules. A webhook
sender that targets arbitrary public hosts needs a rule for TCP 443 to
`0.0.0.0/0` with the private ranges listed under `except`, so the policy and
`PublicOnly` block the same addresses. An operator endpoint needs a rule for its
namespace and port and nothing wider. Put the rule on the process that sends: a
worker that delivers webhooks should not open egress for the API.

## Put every route behind the rate limiter

`baukit-ratelimit` runs token buckets in two scopes. The identity scope keys on
the subject of a verified `baukit_auth::Principal`. The IP scope keys on the
client address and is the safety net for requests without a principal. Every
route sits inside the limiter, including OAuth callbacks, webhooks and
`/platform/*` routes merged late.

The generated backend wires this in
`templates/backend/backend/crates/__app__-bin/src/rate_limits.rs`.
`protect_routes` applies the IP scope first, then `establish_principal`, then
the identity scope, so junk bearer tokens are throttled per client before any
token verification. Routers that verify callers themselves, such as MCP,
browser sessions and signed webhooks, go in the `late` list and get both scopes
without `establish_principal`, which puts them in the IP bucket.

For a stricter quota on part of an authenticated router, add an
`authenticated_route_group`. The template limits item writes this way with a
group named `item_writes` at 30 requests per minute. Group keys use
`group:<name>:`, and the group name becomes the metric `scope` label. Apply
`baukit_http::finalize` outside all limiter layers so 429 responses carry CORS
headers. A limiter outside `finalize` returns a 429 that browsers report as a
network error.

To limit units rather than requests, such as messages per day, use
`FixedWindowAmountBudget` with `FixedWindowBudgetOptions`. It shares the store
and the fail mode semantics. See the crate README for releases and windows.

## Choose the store and the fail mode

`RedisRateLimitStore::connect_if_enabled` returns `None` when both scopes are
disabled and opens no connection. With a scope enabled it needs a reachable
Redis. A `redis://` URL selects a single node. A `redis+sentinel://` URL
selects Sentinel mode, with the master name as the only path segment.
`InMemoryRateLimitStore` is bounded and per process, so it suits tests and local
runs only.

`fail_mode` decides what happens when the store cannot answer. `open`, the
default, logs the error and lets the request through, so a Redis outage turns
limiting off. `closed` rejects the request. Pick `closed` where abuse costs more
than downtime, for example a route that sends email or SMS. The
`BaukitRateLimitStoreErrors` alert fires on `outcome="error"` in either mode, see
[the rate-limit runbook](../runbooks/app/rate-limit-store-errors.md).

Outside `local`, config validation fails when both scopes are disabled.

## Count your trusted proxies

`trusted_proxy_hops` is the number of proxies in front of the service that
append to `X-Forwarded-For`. The default 0 ignores the header and keys on the
socket peer. With `n` hops the limiter parses only the entry at `len - n`, so
entries a client wrote to the left of it are never read. A value larger than the
real number of appending proxies lets a client choose its own bucket. Write down
which proxies the number counts next to the setting.

The IP scope reads `ConnectInfo<SocketAddr>`. `baukit-runtime` listeners provide
it. A custom server must use
`into_make_service_with_connect_info::<SocketAddr>()`, otherwise requests get a
500 instead of a shared bucket.

IPv6 clients share one bucket per network of `ipv6_prefix_len` bits, default 64,
valid 1 to 128. One /64 is usually one household or one VM. IPv4 and
IPv4-mapped addresses always bucket per address.

## Namespace keys on a shared Redis

Several products can share one Redis. `redis_key_namespace` puts a per-product
prefix in front of every key the Redis store touches: token buckets, amount
budgets and amount releases. A non-empty value must end with `:` and contain no
whitespace or `* ? [ ]`, so a Redis ACL can grant exactly `~<namespace>*`. The
crate README lists the ACL commands a tenant user needs.

The chart sets this for you. With `backing.redis.mode` set to `shared` or
`dedicated`, it sets `<PREFIX>__RATE_LIMIT__REDIS_KEY_NAMESPACE` on the API to
`backing.redis.keyNamespace`, default `<product>-<deploymentEnvironment>:`, so
testing and production on one Redis never share buckets. `shared` labels the
API `baukit.dev/redis-client` and opens egress to the `platform-redis`
namespace on 6379 and 26379. `dedicated` deploys the chart's own Redis and sets
the URL. The worker gets no Redis unless `worker.redis.enabled` is true.

## Settings per environment

The keys live under `rate_limit` in `BaukitConfig`, read from
`<PREFIX>__RATE_LIMIT__<KEY>` with `__` between nested parts, for example
`<PREFIX>__RATE_LIMIT__IDENTITY__REQUESTS_PER_PERIOD`. Defaults come from
`rust/crates/baukit-config/src/rate_limit.rs`.

| Key | Default | Local | Testing, staging, production |
|---|---|---|---|
| `redis_url` | `redis://127.0.0.1/` | compose Redis, `redis://redis:6379/` in `templates/backend/compose.yaml` | from `config.existingSecretRefs` or `backing.redis` |
| `identity.enabled` | `true` | `true` | `true` |
| `identity.requests_per_period` | `60` | default | tune per product |
| `identity.period` | `60` (seconds) | default | default |
| `identity.burst` | `10` | default | tune per product |
| `ip.enabled` | `true` | `true` | `true` |
| `ip.requests_per_period` | `6000` | default | tune per product |
| `ip.period` | `60` (seconds) | default | default |
| `ip.burst` | `500` | default | default |
| `fail_mode` | `open` | `open` | `open`, or `closed` for abuse-sensitive products |
| `trusted_proxy_hops` | `0` | `0`, nothing in front | `1` behind Traefik, set in `templates/backend/deploy/values.yaml`; at most 32 |
| `ipv6_prefix_len` | `64` | default | default |
| `key_prefix` | `rl:` | default | default |
| `redis_key_namespace` | empty | empty | `<product>-<deploymentEnvironment>:`, set by the chart |

At least one of `identity.enabled` and `ip.enabled` must stay true outside
`local`. The identity default sits far below the IP default on purpose: one
NAT address can carry many signed-in users.

The chart also offers `ingress.rateLimitMiddleware`, which attaches a Traefik
middleware in front of the service. It is coarse edge protection and does not
replace the in-process limiter.

## Reference

- [`rust/crates/baukit-egress/README.md`](../../rust/crates/baukit-egress/README.md)
- [`rust/crates/baukit-ratelimit/README.md`](../../rust/crates/baukit-ratelimit/README.md)
- [`rust/crates/baukit-config/src/rate_limit.rs`](../../rust/crates/baukit-config/src/rate_limit.rs)
- [`deploy/chart/baukit-app/README.md`](../../deploy/chart/baukit-app/README.md)
- [`templates/backend/backend/crates/__app__-bin/src/rate_limits.rs`](../../templates/backend/backend/crates/__app__-bin/src/rate_limits.rs)
- [`fixtures/egress/address-policy-v1.json`](../../fixtures/egress/address-policy-v1.json)
- [Migration: rate limiting client IP and buckets](../migrations/rate-limit-client-ip-and-buckets.md)
- [Runbook: rate-limit store errors](../runbooks/app/rate-limit-store-errors.md)
- [ADR 0015: request admission](../adr/0015-request-admission.md)
- [ADR 0016: outbound HTTP through egress](../adr/0016-outbound-http-through-egress.md)
- [Deploying](deploying.md)
- [Observability](observability.md)
