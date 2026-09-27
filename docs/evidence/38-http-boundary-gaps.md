# Item 2 evidence: HTTP boundary gaps

This note covers item 2 of the [cross-product feature plan](../cross-product-feature-plan.md):
exposed CORS headers, a default response cache policy, and bounded cursors that domain crates can
use without Axum.

## Source revisions

Baukit baseline: `ef6ac1a`. Products were re-read at Eigenruhe `f74cebb` (the plan cites
`e44ff88`; the relevant code is unchanged in intent), Hebkit `841bf5d`, Leitbild `bd38b33`,
Redemut `a782538`, Runtime Analyzer `d47bfd5`, Schlauzug `fb280df`, Solo Leveling System `3461eaf`,
and Tiefgang `2d37a06`. Schlauzug's `schlauzug-api/src/lib.rs` and `schlauzug-bin/src/bin/api.rs`
have uncommitted changes, and its `external_http_policy` exists only in that diff, so its lines below
are provisional.

## Source product files

Product paths are relative to `/home/patrick/projects/<product>/backend/crates/`.

- Eigenruhe `eigenruhe-api/src/lib.rs:409-433` (`api_response_contract`)
- Hebkit `hebkit-api/src/adapters/http/mod.rs:180-220` and `http_policy.rs:4-15`
- Leitbild `leitbild-api/src/lib.rs:306-345` (`response_contract_headers`)
- Redemut `redemut-api/src/lib.rs:105-170` (second `CorsLayer`, `private_cache_policy`)
- Runtime Analyzer `finops-api/src/lib.rs:89-119` (`no_store`, `expose_response_headers`)
- Schlauzug `schlauzug-api/src/lib.rs:637-662` (`external_http_policy`, provisional) and
  `schlauzug-bin/src/bin/api.rs:283-310`
- Solo Leveling System `sl-api/src/lib.rs:116-167` (`contract_response_headers`) and
  `sl-domain/src/pagination.rs`
- Tiefgang `tiefgang-api/src/http_policy.rs:8-28` (`response_policy`)

## Observed failure or repeated glue

`baukit-http` exposed only `x-request-id`, `traceparent`, and `tracestate`, although
`baukit-ratelimit` emits `Retry-After`, `RateLimit-Limit`, `RateLimit-Remaining`, and
`RateLimit-Reset`. A browser script could see a 429 but not when to retry. All eight products patch
this after `baukit_http::finalize`: six overwrite or append `Access-Control-Expose-Headers` in a
middleware, and Hebkit and Redemut add a second `CorsLayer`. Every one of them also sets
`Cache-Control` by hand, as `private, no-store` (Eigenruhe, Hebkit, Redemut, Runtime Analyzer) or
`no-store` (Leitbild, Schlauzug, Solo Leveling System, Tiefgang), usually with a path exception for
an OpenAPI document or a public manifest.

`Cursor::decode` base64-decoded and parsed input of any length. Solo Leveling System copied the
whole pagination module into `sl-domain` (with `MAX_CURSOR_BYTES = 4096`) because its domain crate
cannot depend on `baukit-http` without Axum. Redemut wrote its own `HistoryCursor` with a 1024-byte
bound. Eigenruhe, Hebkit, and Tiefgang import `baukit_http` pagination types into their ports,
services, and PostgreSQL adapters, which pulls Axum into those crates.

The generated backend template applied `baukit_ratelimit::layers` and `establish_principal` after
`finalize`. Their 401 and 429 responses therefore skipped Baukit's CORS layer, request lifecycle,
and cache policy. A browser reports such a 429 as a network error, so exposing the headers alone
would not have met the acceptance rule. Seven products already rate-limit inside `finalize`;
Schlauzug's global limiter sits outside (`schlauzug-bin/src/bin/api.rs:305-310`).

## Baukit owner

`baukit-http` owns the exposed-header set and the response cache policy. `baukit-core` owns the
pagination types behind its `pagination` feature. `baukit-http` keeps
`From<PaginationError> for ApiError`. The backend template owns the reference composition order.

## Public types and errors

- `HttpOptions::with_additional_exposed_headers` and `HttpOptions::additional_exposed_headers`.
  Invalid names return `HttpOptionsError::InvalidHeaderName`; duplicates are ignored.
- Default exposed set: `x-request-id`, `traceparent`, `tracestate`, `retry-after`,
  `ratelimit-limit`, `ratelimit-remaining`, `ratelimit-reset`. `ETag` and `Location` wait for
  item 6. The names are private constants in `baukit-http`, because `baukit-ratelimit` depends on
  `baukit-http` and not the reverse. The `baukit-ratelimit` test
  `browsers_can_read_rate_limit_headers_through_the_http_layers` asserts that every header the
  limiter emits is exposed, so the two lists cannot drift silently.
- `ResponseCachePolicy::{PrivateNoStore, HandlerOwned}`, `HttpOptions::with_response_cache_policy`,
  and `HttpOptions::response_cache_policy`. `PrivateNoStore` is the default.
- `baukit_core::pagination::{Cursor, Page, PageKey, PageParams, PaginationError,
  DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, MAX_CURSOR_BYTES}` behind the `pagination` feature.
- The template API crate now exports `routes` and `finalize_api` next to `router`.

## Decisions

### Cursor home: `baukit-core` feature

The plan allowed either `baukit-core` or an Axum-free feature of `baukit-http`. Every other
`baukit-http` module uses Axum, so an Axum-free feature there would make nearly the whole crate
conditional for one module. `baukit-core` already holds framework-free vocabulary and is a
dependency of most Baukit crates. The module needs `base64`, `ring`, and `uuid`, and `ring`
compiles C and assembly, which breaks the crate's promise of three dependencies. The module is
therefore gated behind an opt-in `pagination` feature. Default consumers such as `baukit-config`
and `baukit-telemetry` build unchanged, and domain crates opt in with one line.

### Cursor bound on both sides

`Cursor::decode` checks `encoded.len() > MAX_CURSOR_BYTES` before base64 decoding. The value is
4096 bytes, matching Solo Leveling System. `Cursor::encode` also refuses to issue a longer cursor.
Otherwise a long sort value could produce a cursor that the same library rejects on the next
request. Tests cover the guard at 4096 and 4097 bytes, a valid cursor of exactly 4096 bytes, a
valid cursor of 4098 bytes (unpadded base64 has no length of 4n + 1), and encode above the bound.

### Cache policy scope

The policy runs in the request lifecycle middleware after the handler, so it covers handler
responses, `ApiError` rejections, 404 and 405 fallbacks, body-limit, timeout, and panic envelopes,
CORS preflights, and any authentication or rate-limit layer added before `finalize`. It inserts
`Cache-Control: private, no-store` only when the response has no `Cache-Control` header, so a
handler that sets a value wins. A preflight carrying `no-store` is harmless, because browsers cache
preflights through `Access-Control-Max-Age`.

The `baukit-ops` health, readiness, build-info, and metrics routes live on the separate operations
listener and never pass through `baukit-http` layers, so the policy does not reach them. The
template and the minimal example serve them that way, and neither mounts ops routes on the public
router. If a product merged them into a finalized router, `no-store` would still be the right
value for probes and scrapes. Public documents such as `openapi.json` or a content manifest set
their own `Cache-Control` in the handler, which is how Hebkit, Redemut, Schlauzug, and Tiefgang
already treat those paths.

### Template composition

The template now builds unfinalized `routes`, adds the route-group limiter, the global limiter, and
`establish_principal`, and calls `finalize_api` last. The auth conformance test sends cross-origin
requests and asserts that the 429 carries `Access-Control-Allow-Origin`,
`Cache-Control: private, no-store`, `Retry-After`, and an exposed `retry-after`.

## Product-owned inputs

Products own extra exposed headers (`WWW-Authenticate`, `Deprecation`, `Sunset`, `Link`,
`Content-Language`, `Server-Timing`, `x-next-cursor`, and similar), per-route cache values for
public documents, the decision to use `HandlerOwned`, filter normalization, and sort columns.

## Supported runtimes

All Rust targets supported by `baukit-http`, Axum 0.8, and tower-http 0.7, with Rust 1.95 or newer.
`baukit_core::pagination` needs no async runtime or HTTP framework.

## Failure behavior

- An invalid exposed header name fails option construction with `InvalidHeaderName`.
- An oversized cursor returns `PaginationError::InvalidCursor`, which maps to the existing 400
  `validation_failed` error with a `cursor` field detail. No bytes are decoded or allocated for it.
- A cursor that would exceed the bound fails `Cursor::encode` and `Page::from_rows` with the same
  error.

## Privacy boundary

`private, no-store` keeps per-user responses out of shared and browser caches by default. Exposed
headers carry only request identity, trace context, and quota numbers. Cursor errors never echo the
submitted cursor.

## Breaks

- The `baukit_http::pagination` module and its root re-exports are gone. No re-export remains.
- `Cursor::decode` rejects input over 4096 bytes and `Cursor::encode` refuses to issue it.
- Every response from `finalize` or `layers` now carries `Cache-Control: private, no-store` unless
  a handler set one.
- The default exposed set grows by four headers.
- The `InvalidHeaderName` message changed to `invalid CORS header name`.
- Generated backends: `router` still exists, but the API binary uses `routes` plus `finalize_api`,
  and authentication and rate limiting now run inside the Baukit layers. This is recorded here and
  in the template README because the CLI has no changelog.

## Product adoption change

After each product pins the release containing this item:

- Eigenruhe: delete the expose-header overwrite and the `/api/v1` `no-store` insert from
  `api_response_contract` (`eigenruhe-api/src/lib.rs:409-433`), keeping a handler-set
  `Cache-Control` for the content manifest. Replace `animation_descriptor_cache_control`
  (`lib.rs:435-450`) with the default. Move the `baukit_http::pagination` imports in `eigenruhe-api`,
  `eigenruhe-services`, `eigenruhe-postgres`, `eigenruhe-ports`, and `eigenruhe-bin` to
  `baukit_core::pagination`, and drop the Axum path from the domain and port crates. Pass `etag`
  and `location` after item 6 lands.
- Hebkit: delete the second `CorsLayer` (`hebkit-api/src/adapters/http/mod.rs:181-220`) and
  `http_policy::private_api_cache`; set `allow_credentials` and pass `deprecation` and `sunset` to
  `with_additional_exposed_headers`. Set `Cache-Control` in the `openapi.json` handler. Move the
  root pagination imports in `hebkit-services`, `hebkit-api`, `hebkit-ports`, and `hebkit-postgres`.
- Leitbild: delete `response_contract_headers` (`leitbild-api/src/lib.rs:306-345`) and pass
  `content-language`, `deprecation`, `sunset`, and `link` as exposed headers. Move the
  `journal.rs:9-11` pagination imports.
- Redemut: delete the second `CorsLayer` and `private_cache_policy`
  (`redemut-api/src/lib.rs:105-170`); pass `cache-control`, `www-authenticate`, and
  `server-timing` as exposed headers and set `Cache-Control` in the public manifest, bundle, and
  health handlers. `HistoryCursor` (`redemut-services/src/lib.rs:885-1002`) can move to
  `baukit_core::pagination::Cursor` once its user binding is expressed through the filter hash.
- Runtime Analyzer: delete `no_store` and `expose_response_headers` (`finops-api/src/lib.rs:89-119`)
  and the per-handler `no-store` copies in `routes/evidence.rs`, `audit.rs`, and `reports.rs`. Pass
  `x-audit-id`, `www-authenticate`, `cache-control`, `x-next-cursor`, and `x-export-complete` as
  exposed headers. Move the `finops-services` and `finops-api` pagination imports.
- Schlauzug (provisional): delete the expose-header and `no-store` inserts from
  `external_http_policy` (`schlauzug-api/src/lib.rs:637-662`), keep the `WWW-Authenticate`
  challenge, and move the global `baukit_ratelimit::layers` and `establish_oidc` from
  `schlauzug-bin/src/bin/api.rs:305-310` inside `finalize`.
- Solo Leveling System: delete the expose-header append and the `/api/v1` `no-store` insert from
  `contract_response_headers` (`sl-api/src/lib.rs:116-167`), keep the `WWW-Authenticate` realms,
  and pass `www-authenticate` and `cache-control` as exposed headers. Replace
  `sl-domain/src/pagination.rs` with `baukit_core::pagination`.
- Tiefgang: delete `response_policy` (`tiefgang-api/src/http_policy.rs:8-28`) except the public
  `openapi.json` cache value, which moves into its handler. Drop the `x-ratelimit-*` names, which
  Baukit never emits. Move the pagination imports in `tiefgang-api` and `tiefgang-services`.

## Product defects found

- Schlauzug applies its global `baukit_ratelimit::layers` and `establish_oidc` outside `finalize`
  (`schlauzug-bin/src/bin/api.rs:305-310`), so its 401 and 429 responses carry no CORS headers.
- Solo Leveling System merges its MCP router into the already finalized API router
  (`sl-bin/src/bin/api.rs:297`), so MCP routes skip Baukit's request lifecycle, limits, and CORS
  layer and rely on their own `CorsLayer` (`sl-bin/src/compose/mcp.rs:331-348`).
- Hebkit and Redemut stack a second `CorsLayer` outside Baukit's. Check that responses do not end
  up with two `Access-Control-Allow-Origin` values, which browsers reject.
