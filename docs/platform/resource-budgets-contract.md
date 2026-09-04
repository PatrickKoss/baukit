# Resource budget contract

**Status:** Product-facing contract with production measurements and conformance helpers.
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

The mobile template's `metro.config.js` watches the product root so Metro can resolve `limits.json`,
and blocks every sibling directory except `mobile/`.

Count user text as Unicode scalar values after trimming leading and trailing whitespace. Measure a
document as the UTF-8 byte length of its compact JSON serialization. A row cap counts live rows only.
An update to an existing row does not consume another slot, and a soft-deleted row releases its slot.
Define both per-owner and per-parent caps when either scope can grow independently.

Enforce request-body and batch limits before parsing, database work, outbound calls, or other expensive
work. Rate or work budgets use a named time window and define what consumes the budget.

For accepted-change accounting, reserve the submitted amount with
`AmountBudget::consume` before processing, then call `AmountBudget::release`
with the rejected amount. Release affects only the subject's current window,
floors consumed units at zero, and does not carry unused capacity into a later
window.

## 2. Use the production measurements

Rust applications use `baukit_core::limits`. TypeScript applications use
`@baukit/data-contracts/limits`. Both implementations measure trimmed Unicode scalars, compact JSON
UTF-8 bytes, raw bytes, and collection elements. The matching checks accept an allowed amount. A
passing check returns `measured` and `allowed`. A failed check returns or throws an error containing
the same two values. An allowed amount of zero is valid.

Text measurement removes Unicode `White_Space` values only at the beginning and end. It does not
normalize text. Composed `é` counts as one scalar. `e` followed by a combining acute accent counts as
two. A joined emoji sequence counts every emoji, joiner, and variation selector separately. Rust
strings always contain valid Unicode scalar values. TypeScript rejects an unpaired UTF-16 surrogate
with `ResourceMeasurementError.code === "invalid_unicode"`.

Rust compact JSON measurement uses `serde_json` without pretty printing. TypeScript validates the
input and measures the UTF-8 bytes emitted by `JSON.stringify` without a replacer or indentation. It
accepts null, booleans, finite numbers, scalar-only strings, dense arrays, and plain objects. A plain
object may have `Object.prototype` or a null prototype. Only enumerable own string keys are encoded.
Non-enumerable properties are ignored. `JSON.stringify` determines key order, but order does not
change the byte count.

The TypeScript helper rejects these inputs before encoding:

- `undefined`, functions, symbols, and bigints;
- `NaN` and positive or negative infinity;
- sparse arrays;
- symbol-keyed properties and accessor properties;
- class instances, dates, maps, sets, typed arrays, and objects with custom prototypes;
- circular references; and
- unpaired surrogates in values or keys.

`ResourceMeasurementError.code` identifies `unsupported_json_value`,
`non_finite_json_number`, `circular_json_value`, or `invalid_unicode`. Its message does not include
the rejected value or an object key. Products map the code to their own error vocabulary.

Rust accepts byte and collection slices. TypeScript accepts `Uint8Array` and readonly arrays. These
measurements return the slice or array length. A check accepts the exact boundary and fails only when
`measured > allowed`.

All functions are synchronous and keep no shared state, so concurrent calls do not interact. They
perform no I/O and allocate no resource that needs cleanup. The Rust code supports the workspace
MSRV, Rust 1.95. The TypeScript code targets ES2022 and supports Node 24 or newer, current browsers,
and React Native engines with the ES2022 built-ins used by `@baukit/data-contracts`.

## 3. Keep failure behavior stable

Each rejected budget has a stable snake_case reason code and safe structured details. Product code owns
the vocabulary. Adapters must preserve the code instead of replacing it with transport-specific text.

Apply the same rule at every write ingress, including REST, sync ingestion, imports, and local writes.
Background work that accepts equivalent input follows the same rule. Put the validation in shared
product domain or service code when possible.

## 4. Use the conformance helpers

`baukit-test` provides these checks:

- `check_limit_boundaries` builds and validates inputs at `limit - 1`, `limit`, and `limit + 1`.
- `trimmed_text_length` and `compact_document_bytes` remain as compatibility aliases to the
  `baukit-core` production measurements.
- `check_update_at_capacity` fills a scoped fixture to its live-row cap, rejects one extra create, and
  requires an update to succeed.
- `check_soft_delete_capacity_reuse` fills a fixture, rejects one extra create, soft-deletes one row,
  and requires another create to succeed.
- `check_reason_code_conformance` compares already collected error outputs.
- `check_ingress_reason_code_parity` invokes a list of `NamedIngress` functions and applies a
  caller-supplied reason-code extractor.

Implement `LiveRowLimitAdapter` around a clean product fixture bound to one owner or parent. Run the
update and soft-delete helpers on separate fixtures because each helper fills the fixture.

## 5. Acceptance checks

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

## 6. Migration

Rust production code should replace `baukit_test::trimmed_text_length` and
`baukit_test::compact_document_bytes` with the corresponding `baukit_core::limits` functions. The old
`baukit-test` names remain available and call the production implementation.

TypeScript callers should replace local `Array.from(value.trim()).length` and
`TextEncoder().encode(JSON.stringify(value)).byteLength` helpers with the `/limits` subpath. The new
compact JSON helper rejects unsupported input instead of silently omitting it or converting it to
`null`. Products that intentionally accept those inputs must convert them before measurement.

The cross-runtime cases live in `fixtures/limits/resource-budget-measurements-v1.json`. Changes to
either implementation must keep both fixture suites passing.
