# baukit-egress

`baukit-egress` sends HTTP requests to destinations that users supply, such as
webhook URLs and provider endpoints. A plain client can be pointed at
`169.254.169.254`, a database on the private network, or a host name whose DNS
answer flips to a private address after the check. `GuardedClient` refuses all
three.

The crate is opt-in. It is not part of the generated backend template and is not
wired into `baukit_config::BaukitConfig`.

```rust,ignore
use baukit_egress::{EgressOptions, EgressRequest, GuardedClient};

let client = GuardedClient::new(EgressOptions::default())?;
let response = client
    .execute(EgressRequest::post(url).with_headers(headers).with_body(body))
    .await?;
```

## What the client enforces

| Check | Behavior |
|---|---|
| Scheme | `https` only. `http` only under `AddressPolicy::AllowLoopback`, and then only to loopback addresses. |
| URL shape | No user info, no fragment, a host is required. |
| Address literals | `https://10.0.0.1/`, `https://[::ffff:127.0.0.1]/` and `https://2130706433/` are checked before any connection. |
| DNS answers | Every answer must pass the policy. One private answer among public ones rejects the lookup. |
| Pinning | The connection uses the answers that passed the check. A new connection runs a new lookup and a new check. |
| Redirects | Never followed. A `3xx` comes back as `EgressError::Status`. |
| Proxies | `HTTP_PROXY`, `HTTPS_PROXY` and friends are ignored. |
| Time | Separate bounds for the lookup, the TCP and TLS connect, and the whole request including the body. |
| Response size | The body is read in chunks and stops at the limit, with or without `Content-Length`. `ResponseBody::Discard` skips it. |

Defaults are a 3 s lookup, a 5 s connect, a 10 s request, a 1 MiB body, and a
300 s `Retry-After` cap. `EgressOptions` changes them and refuses zero.

## Address policy

`AddressPolicy::PublicOnly` (the default) allows only globally reachable unicast
addresses. The tables follow the IANA IPv4 and IPv6 Special-Purpose Address
registries and the IPv6 Global Unicast assignments:

- IPv4 blocks `0/8`, `10/8`, `100.64/10`, `127/8`, `169.254/16`, `172.16/12`,
  `192.0.0/24`, `192.0.2/24`, `192.88.99/24`, `192.168/16`, `198.18/15`,
  `198.51.100/24`, `203.0.113/24`, multicast `224/4`, and `240/4`.
- IPv6 allows only `2000::/3`, minus `2001::/23`, `2001:db8::/32`,
  `2002::/16`, and the documentation block `3fff::/20`.
- IPv4-mapped addresses (`::ffff:a.b.c.d`) and the NAT64 well-known prefix
  `64:ff9b::/96` are judged by the IPv4 address they carry, so an IPv6-only
  cluster behind DNS64 still reaches public hosts.

A network-specific NAT64 prefix is not recognized. Its addresses fall outside
`2000::/3` or look like ordinary global addresses, so run the client with a
resolver that returns IPv4 answers on such networks.

`AddressPolicy::AllowLoopback` adds loopback, and plain HTTP to loopback, for
local development and tests. An `http` URL whose literal or resolved answers
include anything but loopback fails with
`EgressError::Destination(DestinationRejection::Scheme)` before a connection,
so a public host is still reached over `https` only.
`AddressPolicy::permits_plain_http` answers the same question for one address.
Never enable the policy in a deployed environment.

`fixtures/egress/address-policy-v1.json` pins every decision with address,
URL, and multi-lookup cases, including a DNS answer that changes between two
lookups. Other runtimes that filter addresses should run the same file.

## Response bodies

By default `execute` reads a `2xx` body up to `EgressOptions::max_response_bytes`
and fails with `ResponseTooLarge` above it. A webhook sender does not need the
receiver's answer, so it drops the body unread:

```rust,ignore
use baukit_egress::{EgressRequest, ResponseBody};

let request = EgressRequest::post(url)
    .with_headers(headers)
    .with_body(body)
    .with_response_body(ResponseBody::Discard);
let delivered = client.execute(request).await?;
assert!(delivered.body().is_empty());
```

Any `2xx` then counts as delivered, whatever its size. The connection closes
instead of returning to the pool. Non-`2xx` statuses are classified the same
way in both modes. Their bodies are read only when `with_error_body_limit` is enabled.

To read a product error code, opt in to bounded error bodies:

```rust,ignore
use baukit_egress::{EgressError, EgressOptions, EgressRequest, GuardedClient};
use axum::http::StatusCode;

let options = EgressOptions::default().with_error_body_limit(4096)?;
let client = GuardedClient::new(options)?;
match client.execute(EgressRequest::post(url).with_body(body)).await {
    Ok(response) => tracing::debug!(status = %response.status(), "request succeeded"),
    Err(EgressError::Status { status: StatusCode::SERVICE_UNAVAILABLE, class, .. }) => {
        tracing::warn!(retry_after = ?class.retry_after(), "service unavailable");
    }
    Err(EgressError::Status { status: StatusCode::CONFLICT, body, .. }) => {
        let problem_code = body.as_ref().and_then(|body| body.problem_code());
        tracing::warn!(problem_code, "upstream conflict");
    }
    Err(error) => return Err(error.into()),
}
```

The limit must be nonzero and at most `max_response_bytes`. Both builders check that
constraint, including when lowering `max_response_bytes` after enabling capture.
`ErrorBody::bytes()` returns the captured bytes; `content_type()` returns the response's
content type. `problem_code()` returns the code from Baukit's shared JSON error envelope,
or `None` for other bodies or incomplete JSON.

Reading stops when the limit is filled and sets `truncated()`, without returning
`ResponseTooLarge`. A known-length body that fits exactly is complete. An unknown-length
body that ends exactly at the limit is marked truncated because the client stops without
waiting for more bytes. Reads share the whole request timeout. If reading fails or times
out, the status and retry class are preserved and the captured body is `None`.

## Resolver port

```rust,ignore
pub trait Resolver: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a>;
}
```

`SystemResolver` calls the operating system through Tokio. `StaticResolver`
maps fixed names to fixed answers for tests and for pinned internal
destinations. `GuardedClient::with_resolver` takes any implementation, for
example one backed by a DNS library with its own cache.

## Errors

`EgressError` names what failed and how to react. No variant carries the URL.

| Variant | `code()` | `retry_class()` |
|---|---|---|
| `Destination(DestinationRejection)` | `destination_not_allowed` | `Permanent` |
| `BlockedAddress` | `blocked_address` | `Permanent` |
| `Resolve` | `resolve_failed` | `Unavailable` |
| `Timeout` | `timeout` | `Timeout` |
| `Transport` | `transport_failed` | `Unavailable` |
| `Status { status, class, body }` | `upstream_status` | `classify_http_status_with_options(status, headers, options)` |
| `ResponseTooLarge { limit }` | `response_too_large` | `Permanent` |

`Status` uses `baukit_http::classify_http_status_with_options`. A `429` with `Retry-After`
comes back as `RetryClass::RetryAfter` and without it as `RateLimited`. Every `5xx`,
including `504`, with a valid `Retry-After` also becomes `RetryAfter`. Without a usable
header, `504` is `Timeout` and other `5xx` are `Unavailable`. `408` is `Timeout`,
`425` is `Unavailable`, `401` and
`403` are `Revoked` by default, and every other status, `3xx` included, is `Permanent`.
The body of a non-`2xx` response is not read by default. `error_body()` returns it
when capture is enabled.

Enable `EgressOptions::with_forbidden_rate_limit()` for APIs such as GitHub that report
rate limits with `403`. A usable `Retry-After` gives `RetryAfter`. Otherwise,
`x-ratelimit-remaining: 0` uses `x-ratelimit-reset` as a Unix epoch in seconds,
or gives `RateLimited` without a usable reset. Past resets give a zero delay.
Other `403` responses and all `401` responses stay `Revoked`.
`EgressOptions::forbidden_rate_limit()` reports whether the option is enabled.

The delay in `RetryAfter` is the receiver's value, clamped to
`EgressOptions::max_retry_after` (300 s by default). A receiver that sends
`Retry-After: 86400` gets its next attempt after five minutes, not a day. Set
`with_max_retry_after` to change the cap.

## Privacy

Every request runs inside an `egress.request` span with the method, the host,
the port, the response status, and `error.type`. Failures log one debug event
with the error code and, when captured, the Baukit problem code. Neither contains raw
error bytes, the path, the query, user info, or
header values, and `EgressRequest`'s `Debug` output shows only the method, the
host, the header names, and the body length. Transport errors drop the URL
before they are wrapped. Webhook URLs often embed tokens in the path or query,
so keep it that way when you add fields.

Error bodies may contain personal data. `EgressError`'s `Display` never includes the body.
`ErrorBody`'s `Debug` shows only its length, truncation flag, and problem code, and
`EgressError`'s `Debug` uses that representation. Log the problem code instead of the bytes.
