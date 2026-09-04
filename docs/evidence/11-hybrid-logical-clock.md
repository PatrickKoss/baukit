# Hybrid logical clock evidence

## Source product files

- `/home/patrick/projects/redemut/backend/crates/redemut-services/src/hlc.rs`
- `/home/patrick/projects/redemut/backend/crates/redemut-services/src/lib.rs`
- `/home/patrick/projects/redemut/packages/sync/src/hlc.ts`
- `/home/patrick/projects/redemut/packages/sync/test/hlc.test.ts`
- `/home/patrick/projects/redemut/packages/sync/test/hlc-vectors.test.ts`
- `/home/patrick/projects/redemut/testdata/hlc-vectors/vectors.json`

## Observed repeated glue

Redemut maintains matching Rust and TypeScript clocks. Both encode physical milliseconds and a
logical counter into one JavaScript-safe integer. The product also maintains one fixture corpus to
keep observation and rollover behavior equal across runtimes.

## Baukit owner

`baukit_sync::hlc` owns the Rust clock. `@baukit/sync-client/hlc` owns the TypeScript clock. The
shared vectors live in `fixtures/hlc/vectors-v1.json`.

## Public types and errors

Rust publishes `HybridLogicalClock`, `HybridLogicalClockState`, `HlcError`, `encode`, `decode`, and
`compare`. TypeScript publishes the matching clock, state, encode, decode, and compare operations,
plus `HlcStorage`, `HlcPhysicalClock`, and `HybridLogicalClockError`. Stable TypeScript error codes
cover invalid components, timestamps, physical time, blank device IDs, and safe-integer exhaustion.

## Product-owned inputs

Products supply physical time, device IDs, state persistence, merge rules, compare-and-swap loops,
cursor protocols, schemas, and conflict tie-breaks. Baukit does not generate device IDs or select a
winning record.

## Cases

- Concurrency: TypeScript serializes calls when storage is configured and commits state after each
  successful write.
- Failure: corrupt state resets to zero. Invalid input and safe-integer exhaustion leave state
  unchanged. Storage failures reject unchanged and do not stop the call queue.
- Privacy: state contains only physical time, a logical counter, and the caller-supplied device ID.
  Errors contain no state or device ID.
- Cleanup: the clocks own no handles or subscriptions. Products own storage deletion when an
  identity or local database is removed.

## Supported runtimes

Rust 1.95 or newer. TypeScript targets ES2022, including Node 24 or newer, browsers, and compatible
React Native JavaScript engines.

## Product adoption change

A Redemut adoption pull request can delete
`backend/crates/redemut-services/src/hlc.rs`, change the service import to `baukit_sync::hlc`,
delete `packages/sync/src/hlc.ts`, and import the client clock from `@baukit/sync-client/hlc`.
Redemut can also delete `packages/sync/test/hlc.test.ts`,
`packages/sync/test/hlc-vectors.test.ts`, and `testdata/hlc-vectors/vectors.json` after retaining
product tests for its merge and persistence adapters. No adoption pull request exists because the
product repository is read-only for this batch.
