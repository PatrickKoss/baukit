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

## Errors say the same thing every time

`ApiError` produces the `{ "error": { "code", "message", "request_id", "details" } }` envelope from
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

`PageParams`, `Page`, and `PageKey` implement keyset pagination with opaque cursors that are bound to
the request filters:

```rust
# use axum::extract::Query;
# use baukit_http::{ApiError, Page, PageKey, PageParams, ResponseEnvelope};
# use serde::{Deserialize, Serialize};
# use uuid::Uuid;
# #[derive(Deserialize)]
# struct ListQuery { limit: Option<i64>, cursor: Option<String>, category: Option<String> }
# #[derive(Serialize)]
# struct Filters { category: Option<String> }
# #[derive(Clone, Serialize)]
# struct Item { id: Uuid, name: String }
# #[derive(Serialize)]
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

## Outbound retries

`classify_http_status` turns an upstream response into a `RetryClass` so every outbound client in the
process shares one policy: `RetryAfter(duration)` when the upstream named a delay, `RateLimited` when
it did not, `Unavailable`, `Timeout`, `Revoked` for a rejected credential, and `Permanent`.

`Revoked` is separate from `Permanent` because the recovery differs. A revoked credential needs
re-authorization, and retrying it burns quota against an endpoint that will keep saying no.
`retry_after_from_headers` parses both the delay-seconds and HTTP-date forms.

## Scope

The crate owns the lifecycle around your handlers, not the handlers. No business logic, no
persistence, no authentication; `baukit-auth` layers that on top.
