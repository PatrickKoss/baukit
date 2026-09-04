# Authenticated route-group limit evidence

- Source product files: `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-api/src/rate_limit.rs`, the group composition in `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-api/src/lib.rs`, and store selection in `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-bin/src/bin/api.rs`.
- Observed repeated glue: Eigenruhe builds five authenticated group layers. Each layer constructs subject keys, delegates store calls, applies fail mode, records bounded outcomes, and builds the same headers and retry detail. Its `ApiRateLimitStore` also delegates both Baukit store traits.
- Baukit owner: `baukit-ratelimit` owns token-bucket persistence, Axum enforcement, standard rate-limit responses, and the type-erased shared store.
- Public types and errors: `AuthenticatedRouteGroupOptions`, `AuthenticatedRouteGroupOptionsError`, `authenticated_route_group`, and `SharedRateLimitStore`. Setup rejects empty, unsafe, or names over 64 bytes. Runtime store failures use the configured `RateLimitFailMode`.
- Product-owned inputs: group names, quotas, route grouping, request predicates, subject-key functions, enablement, Redis selection, and product metric names.
- Concurrency, failure, privacy, and cleanup cases: store decisions remain atomic per group and subject key; predicates bypass uncounted requests; fail-open and fail-closed are explicit; subject keys appear only in store keys and never metric labels; in-memory and Redis expiry remain store-owned.
- Supported runtimes: Axum services on Baukit's Rust MSRV with the in-memory or Redis adapter, including Redis Sentinel through the existing store.
- Product adoption change: an Eigenruhe adoption pull request can delete `route_group`, `enforce_route_group`, response normalization, header helpers, metric recording, and `ApiRateLimitStore` from `backend/crates/eigenruhe-api/src/rate_limit.rs`. Its route settings and group assignments remain in the product.
