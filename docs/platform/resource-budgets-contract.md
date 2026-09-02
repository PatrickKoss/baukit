# Resource budget contract

**Status:** Product-facing contract with conformance helpers in `baukit-test`.
**Applies to:** Every write path that accepts, stores, or processes product data.
**Related:** [telemetry specification](./telemetry-spec.md) and [integration reliability](./integration-reliability.md).

## 1. Declare each budget

Every endpoint or write operation records an explicit limit or an explicit decision that a limit does
not apply for:

- request body bytes;
- collection length;
- compact serialized document bytes;
- live rows per owner and, where applicable, per parent;
- bulk batch size; and
- expensive work per time window.

Products own the values and policy. Keep the values in one product-owned source so server, client, and
operations code do not drift. A missing limit needs a written reason. It is not an implicit unlimited
budget.

Count user text as Unicode scalar values after trimming leading and trailing whitespace. Measure a
document as the UTF-8 byte length of its compact JSON serialization. A row cap counts live rows only.
An update to an existing row does not consume another slot, and a soft-deleted row releases its slot.
Define both per-owner and per-parent caps when either scope can grow independently.

Enforce request-body and batch limits before parsing, database work, outbound calls, or other expensive
work. Rate or work budgets use a named time window and define what consumes the budget.

## 2. Keep failure behavior stable

Each rejected budget has a stable snake_case reason code and safe structured details. Product code owns
the vocabulary. Adapters must preserve the code instead of replacing it with transport-specific text.

Apply the same rule at every write ingress, including REST, sync ingestion, imports, and local writes.
Background work that accepts equivalent input follows the same rule. Put the validation in shared
product domain or service code when possible.

## 3. Use the conformance helpers

`baukit-test` provides these checks:

- `check_limit_boundaries` builds and validates inputs at `limit - 1`, `limit`, and `limit + 1`.
- `trimmed_text_length` and `compact_document_bytes` implement the text and document measurements used
  by this contract.
- `check_update_at_capacity` fills a scoped fixture to its live-row cap, rejects one extra create, and
  requires an update to succeed.
- `check_soft_delete_capacity_reuse` fills a fixture, rejects one extra create, soft-deletes one row,
  and requires another create to succeed.
- `check_reason_code_conformance` compares already collected error outputs.
- `check_ingress_reason_code_parity` invokes a list of `NamedIngress` functions and applies a
  caller-supplied reason-code extractor.

Implement `LiveRowLimitAdapter` around a clean product fixture bound to one owner or parent. Run the
update and soft-delete helpers on separate fixtures because each helper fills the fixture.

## 4. Acceptance checks

- Tests cover `limit - 1`, `limit`, and `limit + 1` for body bytes, text, collection length, document
  bytes, and batch size where each applies.
- Updating an existing row at capacity succeeds.
- Soft-deleted rows free capacity.
- Per-owner and per-parent caps use live-row counts and have separate tests.
- REST, sync, import, and local write paths return the same stable reason code for the same rejection.
- Request-body and batch rejection happens before expensive work starts.
- Expensive work has a finite allowance and time window, with tests for exhaustion and reset.
- Metrics use bounded, code-defined labels only. Owner IDs, parent IDs, payload values, URLs, request
  IDs, and error text are forbidden as labels.
