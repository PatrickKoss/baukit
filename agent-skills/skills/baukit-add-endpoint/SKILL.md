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
For a table pulled by offline clients, follow the generated product's `docs/syncable-tables.md` before writing its migration or repository methods.

## Keep the HTTP contract

1. Register the route and use Baukit extractors such as `ApiJson` and `ApiPath` so rejection responses use the standard envelope.
2. Return `ApiError`; never serialize an ad hoc error body. Public codes must be stable snake_case, messages must be safe, validation details must be structured, and internal causes must stay out of responses. The envelope is `ErrorEnvelope { error: ErrorBody { code, message, request_id, details } }`.
3. Document every success and error response with `#[utoipa::path]`. Use `ErrorEnvelope` for error bodies.
4. Add the handler to `ApiDoc`'s `paths(...)`; add new DTOs and error schemas to `components(schemas(...))`.
5. Follow the contracts in the matching Baukit checkout: `docs/platform/baukit-conventions.md`, `docs/platform/telemetry-spec.md`, and `docs/platform/resource-budgets-contract.md`. Do not add endpoint-specific HTTP metrics; `baukit-http` records the standard RED metrics once.

Use the field helpers that `baukit-http` exports:

```rust
return Err(ApiError::validation_field(
    "name",
    "must contain at most 120 characters",
));

return Err(ApiError::validation_fields([
    ("starts_at", "must be before ends_at"),
    ("ends_at", "must be after starts_at"),
]));
```

Use `ApiError::new(...).with_details(...)` when the endpoint needs a stable product code other than `validation_failed`. Keep detail keys stable and values safe for clients.

## Prove ingress parity and replay behavior

Before implementing, write down and test:

1. Length, numeric, chronology, enum, uniqueness, and cross-field bounds.
2. The same invariant at every applicable write ingress: REST, MCP/agent tools, sync ingestion, local persistence, imports, and background jobs. Put the rule in product domain/service code so adapters cannot diverge.
3. A stable snake_case error code and structured details for each client-actionable outcome.
4. One transaction for every compound action, including derived records and outbox rows.
5. Same-tick duplicate activation, client retry, transport timeout, process restart, and replay behavior.
6. An idempotency key representing the user's intent whenever one action may create multiple records. Store and check it in the same transaction as the result. Do not invent a platform-wide expiry/storage policy; that remains product-owned until a shared contract exists.

Add tests at each ingress plus transaction rollback and idempotent replay tests. A UI disabled state is not a same-tick mutex and an HTTP timeout is not proof that a write failed.

For generated TypeScript clients, branch on the typed Baukit error and localize from `code + details`:

```ts
import { isApiError } from "@baukit/api-runtime";

try {
  await createWidget(input);
} catch (error) {
  if (isApiError(error, "validation_failed")) {
    showFieldErrors(error.details);
  } else if (isApiError(error)) {
    showError(resolveApiError(error.code, error.details) ?? error.message);
  } else {
    throw error;
  }
}
```

`message` is safe fallback text, not a stable localization key. Keep resolution and user-facing copy product-owned.

## Set resource budgets

Before implementing a write endpoint, record an explicit limit or an explicit decision that no limit
applies for each of these:

- request body bytes;
- collection length;
- compact serialized document bytes;
- live rows per owner and, where applicable, per parent;
- bulk batch size; and
- expensive work per named time window.

Enforce body and batch limits before parsing, persistence, outbound calls, or other expensive work.
Count text as Unicode scalar values after trimming. Measure documents by the UTF-8 bytes in their
compact JSON encoding. Row caps count live rows. Updates must succeed at capacity, and soft-deleted
rows must release capacity.

When only accepted changes should consume a time-window budget, reserve the submitted batch with
`AmountBudget::consume` before processing. Call `AmountBudget::release` with the rejected amount
afterward. A release applies to the current subject window and floors usage at zero.

Give every rejected budget a stable snake_case reason code with safe structured details. Apply the
same rule and code to REST, sync ingestion, imports, and local writes. Include equivalent background
writes when they accept the same data. Keep the limits, reason codes, persistence, and policy in the
product.

Use `baukit-test` in product tests:

- `check_limit_boundaries` checks payloads built at `limit - 1`, `limit`, and `limit + 1`.
- `trimmed_text_length` and `compact_document_bytes` use the contract's text and document
  measurements.
- `LiveRowLimitAdapter` with `check_update_at_capacity` proves updates do not consume capacity.
- `LiveRowLimitAdapter` with `check_soft_delete_capacity_reuse` proves soft deletion releases a slot.
- `NamedIngress` with `check_ingress_reason_code_parity` invokes each write path and compares the
  reason code returned by the caller-supplied extractor.

Run row checks separately for per-owner and per-parent caps. Metrics for budget rejection or expensive
work may use only bounded, code-defined labels. Never use owner IDs, parent IDs, payload values, URLs,
request IDs, or error text as metric labels.

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
