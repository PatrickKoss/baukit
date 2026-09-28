# baukit-http

`baukit-http` gives every Baukit service one request lifecycle. `finalize` wraps a router with
extractor and routing errors, request identity, W3C trace extraction and propagation, route-template
spans, HTTP RED metrics, panic and timeout envelopes, body and concurrency limits, and explicit CORS.

```rust
use axum::{Router, routing::get};
use baukit_http::{HttpOptions, JsonRejectionCodes, RequestId, finalize};

async fn hello(request_id: RequestId) -> String {
    format!("hello from request {}", request_id.as_str())
}

let options = HttpOptions::default()
    .with_allowed_origins(["https://app.example.com"])?
    .with_additional_allowed_headers(["accept", "x-webhook-secret"])?
    .with_json_rejection_codes(JsonRejectionCodes::default());
let app = finalize(Router::new().route("/hello", get(hello)), options);
# let _: Router = app;
# Ok::<(), baukit_http::HttpOptionsError>(())
```

Defaults are a 2 MiB body limit, 1,024 concurrent requests, and a 30 second timeout. CORS origins
start empty and have to be named; there is no permissive default to forget to tighten.

## Browser-visible headers

Browsers hide response headers from cross-origin scripts unless `Access-Control-Expose-Headers`
names them. The default exposed set is `x-request-id`, `traceparent`, `tracestate`,
`Retry-After`, `RateLimit-Limit`, `RateLimit-Remaining`, `RateLimit-Reset`, `ETag`, and `Location`.
That covers the request identity, every header `baukit-ratelimit` emits, the revision validator,
and the address of a created resource. The default allowed request headers are `Authorization`,
`Content-Type`, `If-Match`, `Idempotency-Key`, `x-request-id`, `traceparent`, and `tracestate`, so
a browser can send a conditional or keyed write. Add product headers with `with_additional_exposed_headers`:

```rust
use baukit_http::HttpOptions;

let options = HttpOptions::default()
    .with_allowed_origins(["https://app.example.com"])?
    .with_additional_exposed_headers(["x-next-cursor"])?;
assert_eq!(options.additional_exposed_headers().len(), 1);
# Ok::<(), baukit_http::HttpOptionsError>(())
```

CORS headers only reach responses produced inside `finalize`. Add authentication and rate-limit
layers to the router before calling `finalize`, or their 401 and 429 responses reach the browser
without CORS headers and look like network errors.

## Cache policy

API responses usually carry per-user data, so `finalize` adds `Cache-Control: private, no-store` to
every response that has no `Cache-Control` header yet. A handler that sets its own value keeps it,
which is how a public document such as an OpenAPI file opts into caching. Products that own the
header everywhere turn the default off:

```rust
use baukit_http::{HttpOptions, ResponseCachePolicy};

let options = HttpOptions::default().with_response_cache_policy(ResponseCachePolicy::HandlerOwned);
assert_eq!(options.response_cache_policy(), ResponseCachePolicy::HandlerOwned);
```

The `baukit-ops` health, readiness, build-info, and metrics routes run on the separate operations
listener without these layers, so the policy does not touch them.

## Errors say the same thing every time

`ApiError` produces the `{ "error": { "code", "message", "requestId", "details" } }` envelope from
`baukit-openapi`, so the documented schema and the actual response body come from one type.

Constructors cover the usual cases: `bad_request`, `validation_field`, `unauthenticated`,
`permission_denied`, `not_found`, `conflict`, `rate_limited`. The interesting one is `internal`:

```rust
use baukit_http::ApiError;

# fn example(error: std::io::Error) -> ApiError {
ApiError::internal(error)
# }
```

It takes ownership of the cause, keeps it for logging, and returns a flat "An internal error
occurred" to the client. Leaking a driver error to a caller is how connection strings, table names,
and internal hostnames end up in someone's browser console. The type makes doing it right the shorter
path.

Use `with_header` to add response headers without wrapping `ApiError`. `with_retry_after` writes a
`Retry-After` value in delta seconds, which covers the common quota response:

```rust
use axum::http::{HeaderValue, header};
use baukit_http::ApiError;

# fn quota_error() -> ApiError {
ApiError::rate_limited()
    .with_retry_after(30)
    .with_header(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
# }
```

If code adds `X-Request-Id` through `with_header`, the request middleware replaces it with the
request's actual ID.

Extractor and routing failures produce the same envelope. A malformed JSON body should not return
Axum's default plain-text rejection while every other error on the service returns structured JSON;
clients then need two parsers for one API.

### JSON rejection classes

`ApiJson<T>` can retain the reason Axum rejected a JSON body. Enable this behavior with
`HttpOptions::with_json_rejection_codes`. The default class codes are:

| Rejection | Status | Default code | Safe detail |
| --- | ---: | --- | --- |
| Body exceeds the configured limit | 413 | `payload_too_large` | None |
| Missing or invalid JSON content type | 415 | `unsupported_media_type` | None |
| Malformed JSON | 400 | `invalid_json` | `body: must contain valid JSON` |
| JSON does not match the target type | 422 | `validation_failed` | `body: must match the request schema` |

Use `JsonRejectionCodes::new` when a product already has different public codes:

```rust
use baukit_http::{HttpOptions, JsonRejectionCodes};

let codes = JsonRejectionCodes::new(
    "payload_too_large",
    "invalid_content_type",
    "malformed_payload",
    "invalid_payload",
)?;
let options = HttpOptions::default().with_json_rejection_codes(codes);
# let _ = options;
# Ok::<(), baukit_http::HttpOptionsError>(())
```

Baukit never copies the submitted body, Axum rejection text, or serde parser details into the
response or request logs. Products map the stable codes to their own user-facing text. Products also
set route-specific body limits, for example with Axum's `DefaultBodyLimit`.

### Migration from one JSON rejection code

No change is required for current consumers. `HttpOptions::default()` and
`with_json_rejection_code("invalid_json")` keep the previous behavior: every `ApiJson<T>` rejection
returns status 400 with the one configured code. This compatibility mode remains available for this
release cycle.

To distinguish oversized bodies and content-type failures, replace `with_json_rejection_code` with
`with_json_rejection_codes`. Review clients for the new 413, 415, and 422 statuses before making the
switch. The global `HttpOptions::body_size_limit` uses the configured body-too-large code in
class-specific mode.

## Request locale extraction

`RequestLocale` selects only from product-configured locales. Put `RequestLocaleConfig` in Axum
state directly, or derive `FromRef` for a field in the application state.

```rust
use axum::{Router, routing::get};
use baukit_http::{LocaleQueryOverride, RequestLocale, RequestLocaleConfig};

async fn greeting(locale: RequestLocale) -> String {
    format!("locale={}", locale.as_str())
}

let locale_config = RequestLocaleConfig::new(
    ["en", "de", "es-MX"],
    "en",
    LocaleQueryOverride::parameter("locale")?,
)?;
let app = Router::new()
    .route("/greeting", get(greeting))
    .with_state(locale_config);
# let _: Router = app;
# Ok::<(), baukit_http::RequestLocaleConfigError>(())
```

An enabled, percent-decoded query override wins over `Accept-Language`. An unsupported explicit
query locale is a 400 validation error. Header choices use the highest quality value, then header
order for equal values. Locale lookup checks an exact configured tag, a configured regional tag for
a bare language, then progressively shorter requested tags. Configuration order resolves multiple
matches for one range. A wildcard selects the first configured locale. Missing or unmatched headers
use the configured fallback.

Malformed percent escapes, duplicate override parameters, malformed language ranges or quality
values, and oversized inputs return a 400 `validation_failed` envelope. The raw query limit is 2,048
bytes and the combined `Accept-Language` limit is 1,024 bytes. The extractor does not log or return
submitted values.

This API is additive. Existing handlers keep their current locale behavior until they add
`RequestLocaleConfig` to state and accept `RequestLocale`. Product locale lists and translated copy
remain outside `baukit-http`.

## Request identity and tracing

Every request carries a `RequestId`, echoed in `X-Request-Id` and available as an extractor. It goes
into the error envelope too, so a user reporting a failure hands you the exact ID to grep for.

`extract_trace_context` reads inbound W3C trace headers and `inject_trace_context` puts the current
context on an outbound request, which is what keeps one trace intact across service hops. Spans are
named by route template rather than by concrete path, so `/widgets/{id}` is one span name instead of
one per widget.

## Metrics

The crate records `http_requests_total`, `http_request_duration_seconds`, and
`http_requests_in_flight` through the `metrics` facade and never installs a recorder. The recorder
owner, normally `baukit-telemetry`, configures the duration histogram with `DURATION_BUCKETS`.

One recorder per process is the reason for that split. Two crates each installing their own is a
runtime conflict, and buckets configured in two places drift apart.

## Keyset pagination

`baukit_core::pagination` owns `PageParams`, `Page`, `PageKey`, and `Cursor`, which implement keyset
pagination with opaque cursors bound to the request filters. They live in `baukit-core` behind its
`pagination` feature, so domain and service crates can build pages without depending on Axum.
`baukit-http` enables that feature and converts `PaginationError` into a field-level `ApiError`:

```rust
# use axum::extract::Query;
# use baukit_core::pagination::{Page, PageKey, PageParams};
# use baukit_http::{ApiError, ResponseEnvelope};
# use serde::{Deserialize, Serialize};
# use uuid::Uuid;
# #[derive(Deserialize)]
# struct ListQuery { limit: Option<i64>, cursor: Option<String>, category: Option<String> }
# #[derive(Serialize)]
# struct Filters { category: Option<String> }
# #[derive(Clone, Serialize)]
# struct Item { id: Uuid, name: String }
# #[derive(Serialize)]
# #[serde(rename_all = "camelCase")]
# struct PageMeta { next_cursor: Option<String> }
async fn list(
    Query(query): Query<ListQuery>,
) -> Result<ResponseEnvelope<Vec<Item>, PageMeta>, ApiError> {
    let params = PageParams::new(query.limit, query.cursor)?;
    let filters = Filters { category: query.category };
    let after = params.decode_cursor(&filters)?;

    // Fetch `params.fetch_limit()?` rows ordered by (name, id), starting after
    // `after.page_key::<String>()?` when it is present.
    # let _ = after;
    let rows: Vec<Item> = Vec::new();

    let page = Page::from_rows(rows, &params, &filters, |item| {
        PageKey::new(item.name.clone(), item.id)
    })?;
    Ok(ResponseEnvelope::new(page.items, PageMeta { next_cursor: page.next_cursor }))
}
# let _ = list;
```

Binding the cursor to the filters is what makes it safe. A cursor from a `category=books` query
replayed against `category=tools` is rejected instead of paging through the wrong result set from a
meaningless offset. Keyset beats `OFFSET` for the usual reason: page 500 costs the same as page 1,
and rows inserted mid-scroll do not shift everything down by one.

`Cursor::decode` rejects input longer than `MAX_CURSOR_BYTES` (4096) with
`PaginationError::InvalidCursor` before it decodes anything, and `Cursor::encode` refuses to issue a
cursor above that bound.

## Revision preconditions

A write that must not overwrite someone else's change carries the ETag from the last read in
`If-Match`. `RevisionEtag` formats and parses strong ETags of the form `"<prefix><revision>"`. The
prefix is the product's, such as `rev-` or `settings-`, and may be empty. It is compared byte for
byte, so `REV-4` does not match `rev-`. The revision is a `Revision`, a non-negative integer up to
`i64::MAX` in canonical decimal: `0` is valid, `01`, `+1`, and `-0` are not.

```rust
use axum::http::{HeaderMap, StatusCode, header};
use baukit_http::{ApiError, Revision, RevisionEtag, ensure_current_revision};

const PLAN_ETAG: RevisionEtag<'static> = RevisionEtag::new("rev-");

async fn update_plan(headers: HeaderMap) -> Result<(StatusCode, HeaderMap), ApiError> {
    let expected = PLAN_ETAG.required_if_match(&headers)?;
    let stored = Revision::try_from(7_i64)?;
    ensure_current_revision(expected, stored)?;
    let mut response = HeaderMap::new();
    response.insert(header::ETAG, PLAN_ETAG.header_value(Revision::try_from(8_i64)?));
    Ok((StatusCode::OK, response))
}
# let _ = update_plan;
```

Each route picks `required_if_match` or `optional_if_match`. The parser accepts exactly one strong
ETag, with surrounding spaces and tabs trimmed. Every failure is a `PreconditionError` that
converts into `ApiError`:

| Case | Status | `code` | `details` |
| --- | --- | --- | --- |
| Required header missing | 428 | `precondition_required` | none |
| `*`, weak ETag, list, repeated header, non-ASCII bytes, wrong prefix, bad revision | 400 | `invalid_if_match` | `reason`, one of the `InvalidIfMatch::reason` values |
| Revision is not the stored one | 412 | `precondition_failed` | `currentRevision` when known |

The comparison against storage stays in the product, because it usually happens inside the
update statement. `ensure_current_revision` turns a mismatch into the 412. When the store only
reports that nothing matched, return `PreconditionError::Stale { current: None }`. A stored value
outside the `Revision` range converts into a 500 through `RevisionOutOfRange`.

`RevisionEtag::new` panics on an invalid prefix, which is a compile error in a `const`. Use
`RevisionEtag::try_new` for a prefix built at runtime, such as one that embeds a resource ID.
`baukit-openapi` documents the header parameter and the `ETag` response header with
`document_if_match` and `document_etag`. The shared vectors live in
`fixtures/etag-preconditions/vectors-v1.json`.

## Idempotency keys

A client that loses the response to a create resends it with the same `Idempotency-Key`, and the
server replays the stored result instead of creating a second row. `IdempotencyKeyRule` parses the
header with bounds the route chooses. Storage, the fingerprint, and the replay stay in the product;
[replay-safe mutations](../../../docs/platform/replay-safe-mutations.md) is the protocol they
follow.

```rust
use axum::http::{HeaderMap, StatusCode};
use baukit_http::{ApiError, IdempotencyError, IdempotencyKeyRule};

const CREATE_NOTE_KEY: IdempotencyKeyRule = IdempotencyKeyRule::new(1, 128);

# enum Claim { New, Replay(StatusCode), Reused }
# fn claim_in_transaction(_key: &str) -> Claim { Claim::New }
async fn create_note(headers: HeaderMap) -> Result<StatusCode, ApiError> {
    let key = CREATE_NOTE_KEY.required(&headers)?;
    match claim_in_transaction(key.as_str()) {
        Claim::Replay(stored_status) => Ok(stored_status),
        Claim::Reused => Err(IdempotencyError::Reused.into()),
        Claim::New => Ok(StatusCode::CREATED),
    }
}
# let _ = create_note;
```

Each route picks `required` or `optional`. An optional route with no header returns `Ok(None)` and
runs without a replay record. A key is one header of `min..=max` bytes of visible ASCII, and `max`
is at most `MAX_IDEMPOTENCY_KEY_BYTES` (255). The value is opaque. Nothing is trimmed, and quotes
are part of it. `IdempotencyKey`'s `Debug` output hides the value. `IdempotencyKeyRule::new`
panics on bad bounds, which is a compile error in a `const`; `try_new` returns
`InvalidIdempotencyKeyRule` instead.

| Case | Status | `code` | `details` |
| --- | --- | --- | --- |
| Required header missing | 400 | `idempotency_key_required` | none |
| Repeated header, empty, too short, too long, or a byte outside visible ASCII | 400 | `invalid_idempotency_key` | `reason`, one of the `InvalidIdempotencyKey::reason` values |
| Same key and scope with a different fingerprint | 409 | `idempotency_key_reused` | none |
| The first request with the key is still running | 409 | `idempotency_key_in_progress` | none |

The product returns `IdempotencyError::Reused` or `InProgress` from its replay lookup. The
`baukit-test` replay-safe mutation check proves that lookup against real PostgreSQL.

## Outbound retries

`classify_http_status` turns an upstream response into a `RetryClass` so every outbound client in the
process shares one policy: `RetryAfter(duration)` when the upstream named a delay, `RateLimited` when
it did not, `Unavailable`, `Timeout`, `Revoked` for a rejected credential, and `Permanent`.

`Revoked` is separate from `Permanent` because the recovery differs. A revoked credential needs
re-authorization, and retrying it burns quota against an endpoint that will keep saying no.
`425 Too Early` is `Unavailable`: RFC 8470 section 5.2 lets a client retry once the request is no
longer sent as early data. `retry_after_from_headers` parses both the delay-seconds and HTTP-date
forms.

The delay is uncapped by default, so the classifier reports what the upstream sent. A worker that
honors `Retry-After: 86400` sleeps for a day, so a client that schedules its own retries sets a
cap. Delays above it are clamped to it:

```rust
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER};
use baukit_http::{RetryClass, RetryHeaderOptions, classify_http_status_with_options};

let mut headers = HeaderMap::new();
headers.insert(RETRY_AFTER, HeaderValue::from_static("86400"));
let options = RetryHeaderOptions::default().with_max_retry_after(Duration::from_secs(300));
assert_eq!(
    classify_http_status_with_options(StatusCode::TOO_MANY_REQUESTS, &headers, options),
    RetryClass::RetryAfter(Duration::from_secs(300))
);
```

`baukit_egress::GuardedClient` applies a 300 second cap by default.

## Scope

The crate owns the lifecycle around your handlers, not the handlers. No business logic, no
persistence, no authentication; `baukit-auth` layers that on top.
