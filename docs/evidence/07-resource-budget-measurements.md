# Evidence for production resource-budget measurements

## Source product files

- Eigenruhe `backend/crates/eigenruhe-domain/src/limits.rs`
- Eigenruhe `mobile/src/limits.ts` and `mobile/src/limits.test.ts`
- Eigenruhe `mcp/src/limits.ts` and `mcp/test/limits.test.ts`
- Eigenruhe `limits.json`

## Observed repetition

Eigenruhe counts trimmed Unicode code points, compact JSON UTF-8 bytes, and collection elements in
both Rust and TypeScript write paths. `baukit-test` also contained Rust copies of the text and JSON
measurements, so shipped Rust code could not use Baukit without a development dependency.

## Baukit owner

`baukit-core::limits` owns Rust measurements. `@baukit/data-contracts/limits` owns TypeScript
measurements. `baukit-test` owns conformance only and aliases its old measurement names to
`baukit-core`.

## Public types and errors

Rust publishes `LimitMeasurement`, `LimitExceeded`, `CompactJsonLimitError`, four measurements, and
their checks. TypeScript publishes `LimitMeasurement`, `LimitExceededError`,
`ResourceMeasurementError`, `ResourceMeasurementErrorCode`, four measurements, and their checks.

## Product-owned inputs

Products retain limit values, policy schemas and files, operation mappings, fields, reason codes,
translations, UI copy, and remediation.

## Concurrency, failure, privacy, and cleanup

The functions have no shared state or I/O. Encoding and invalid JavaScript values fail explicitly.
Errors contain counts or stable codes, not measured content or object keys. No cleanup is required.

## Supported runtimes

Rust 1.95 or newer. TypeScript targets ES2022, including Node 24 or newer, current browsers, and
compatible React Native JavaScript engines.

## Product adoption change

An Eigenruhe dependency upgrade can remove `codePointLength`, the local UTF-8 counter, and the local
compact JSON byte functions. Its `limits.json`, parsers, public reason types, and operation mappings
remain product-owned. The adoption pull request has not been opened because the product repository
is read-only for this batch.
