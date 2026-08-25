# Mirrored domain logic workflow

**Status:** Testing workflow; no generic runtime.
**Applies to:** Deterministic product rules intentionally implemented in Rust and TypeScript for server/device parity.

Mirroring is justified when an offline client must make the same deterministic decision as the server. It is not a default requirement for all domain logic.

## 1. Fixtures are the contract

Keep one committed, versioned JSON corpus at a neutral path reachable by both test suites. Each named case contains canonical input and exact expected output; include invalid inputs and boundary cases. Put constants that materially affect results in the fixture contract or assert them explicitly against the fixture version.

Define cross-language representation rules before implementation: timestamps and time zones, decimal/rounding behavior, integer ranges, absent versus `null`, enum strings, object/list ordering, duplicate handling, and deterministic tie-breakers. Do not make fixtures pass by normalizing away a meaningful mismatch in one test harness.

Outputs carry stable machine codes and structured parameters, not English explanation sentences. Localization maps those codes at the UI boundary.

## 2. Keep both implementations honest

- Rust and TypeScript independently parse the same corpus and call their production implementation.
- Both compare the complete canonical result for every case and assert the supported fixture/schema version.
- Both suites run in required CI jobs. A corpus change fails until both implementations agree; do not update only one expected-output copy.
- Add regression cases before fixing a parity defect. Include invalid inputs, every stable code, all rule branches, thresholds on both sides, and ordering/tie cases.
- When implementation constants are generated from fixtures, generate deterministically and fail CI on drift; do not hand-edit the mirror.

The server remains authoritative for persisted outcomes. Generated API clients may type and transport inputs, outputs, codes, and structured parameters, but they do not implement or own the business rules.

## 3. Deliberately product-local

Algorithms, thresholds, product schemas, fixtures, stable domain codes, and the decision to mirror remain in the product. Baukit provides this workflow, not a cross-language rules engine, code generator, or shared business-logic package.

## 4. Acceptance checks

- One fixture file is consumed directly by both production-language test suites.
- Both suites reject an unsupported fixture version and fail on any changed full output.
- The corpus covers invalid input, boundary values, ordering/ties, every branch, and every emitted stable code.
- Locale-independent codes/details cross the API; no English sentence is part of the equality contract.
- A deliberately perturbed Rust result and a deliberately perturbed TypeScript result each fail their required CI job against the same case.
- Rounding, dates/time zones, `null`/absence, and collection order have explicit canonical rules.
