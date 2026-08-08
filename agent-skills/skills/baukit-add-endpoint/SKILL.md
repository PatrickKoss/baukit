---
name: baukit-add-endpoint
description: Add or change an HTTP API endpoint in a Baukit-generated Rust backend while preserving its boundaries, error envelope, Utoipa schema, OpenAPI drift test, and generated TypeScript declarations. Use for new routes, request or response DTOs, service operations, or endpoint contract changes.
---

# Add a backend endpoint

Work from a generated product root whose `baukit.toml` enables `backend`. Extend the existing vertical slice; do not copy template code into the product or hand-edit generated OpenAPI artifacts.

## Preserve the generated boundaries

Use the product name from `baukit.toml` in these paths:

- `backend/crates/<product>-domain/src/lib.rs`: business types and invariants.
- `backend/crates/<product>-ports/src/lib.rs`: repository or integration traits and boundary errors.
- `backend/crates/<product>-services/src/lib.rs`: use cases depending only on domain and ports.
- `backend/crates/<product>-postgres/src/lib.rs`: SQLx adapter; add a migration under `backend/migrations/` when persistence changes.
- `backend/crates/<product>-api/src/lib.rs`: Axum routes, request/response DTOs, service-error mapping, and Utoipa declarations.
- `backend/crates/<product>-bin/src/`: adapter composition and process binaries.

Keep dependency direction domain → ports → services → adapters/API → bin. Add focused tests at the layer where behavior belongs.

## Keep the HTTP contract

1. Register the route and use Baukit extractors such as `ApiJson` and `ApiPath` so rejection responses use the standard envelope.
2. Return `ApiError`; never serialize an ad hoc error body. Public codes must be stable snake_case, messages must be safe, validation details must be structured, and internal causes must stay out of responses. The envelope is `ErrorEnvelope { error: ErrorBody { code, message, request_id, details } }`.
3. Document every success and error response with `#[utoipa::path]`. Use `ErrorEnvelope` for error bodies.
4. Add the handler to `ApiDoc`'s `paths(...)`; add new DTOs and error schemas to `components(schemas(...))`.
5. Follow the contracts in the matching Baukit checkout: `docs/platform/baukit-conventions.md` and `docs/platform/telemetry-spec.md`. Do not add endpoint-specific HTTP metrics; `baukit-http` records the standard RED metrics once.

## Regenerate and verify

From the product root, format, export the schema, run the drift test, and regenerate TypeScript declarations:

```sh
cargo fmt --manifest-path backend/Cargo.toml --all
(cd backend && cargo run --bin openapi -- openapi.json)
(cd backend && cargo test --test openapi_drift)
baukit generate openapi-client
cargo test --manifest-path backend/Cargo.toml
```

`baukit generate openapi-client` writes the schema path and `generated/openapi.d.ts` declared in `baukit.toml`, using pnpm or npx. Update mobile/web call sites against those declarations and commit both generated artifacts. Never edit `backend/openapi.json` or `generated/openapi.d.ts` manually.
