---
name: baukit-mirrored-domain-logic
description: Add, change, or review deterministic product domain logic implemented in both Rust and TypeScript using one versioned JSON fixture corpus as the executable contract. Use when offline clients and servers must make identical decisions, when parity drifts between languages, or when stable reason, warning, or validation codes cross generated API clients.
---

# Mirror domain logic across Rust and TypeScript

Read `<baukit-repo>/docs/platform/mirrored-domain-logic.md`. Mirror only deterministic rules that must run offline and server-side; keep algorithms, thresholds, schemas, codes, and fixtures in the product.

## Establish the fixture contract

1. Put one committed, versioned JSON corpus at a neutral path consumed directly by both test suites.
2. Give every case a diagnostic name, canonical input, and exact expected output. Include invalid input, boundaries on both sides, every branch/code, ordering and tie cases.
3. Specify timestamps/time zones, decimal and rounding behavior, integer range, `null` versus absence, enum strings, duplicate handling, list order, and tie-breakers.
4. Emit stable machine codes plus structured parameters. Keep English explanation text out of the equality contract.

## Implement and gate both sides

Call the production Rust and TypeScript functions from their fixture tests. Assert the supported schema/algorithm version, contract constants, and complete serialized output. Do not maintain separate expected-output copies or weaken one harness with language-specific normalization.

Add the fixture to both required CI jobs. Before fixing a parity bug, add a shared regression case that fails both implementations as applicable. If constants/types are generated, regenerate deterministically and add a drift check.

Generated clients may transport typed inputs, outputs, stable codes, and parameters. They must not reimplement or own the rules; the server stays authoritative for persisted outcomes.

## Verify

Run both languages' format, lint, type/check, and test commands. Deliberately perturb one Rust result and one TypeScript result during test development to prove each required suite fails against the same case, then restore them. Confirm localized clients map codes outside the domain implementation.

Do not extract a Baukit rules engine, copy product fixtures into Baukit, or mirror logic that has no offline/client execution requirement.
