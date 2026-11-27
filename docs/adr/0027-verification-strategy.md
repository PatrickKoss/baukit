# ADR 0027: Each property class gets one verification tool, and every tool runs as a gate step

## Status

Accepted, 2026-11-27.

## Context

Example-based tests check the inputs someone thought of. Baukit has code where
the bug sits outside those inputs: IP and address classifiers, clock and window
arithmetic, decoders of untrusted bytes, and protocols that span transactions,
workers, tabs or devices. A unit test sees one interleaving of a protocol, and
a fuzzer cannot prove an arithmetic bound.

Every formal tool also costs something to write and to run. Picking a tool per
finding would leave a mix nobody maintains. Baukit picks the tool by the class
of property instead, and a proof that no gate runs does not count.

## Decision

| Property class | Tool | Where it lives | Gate step |
|---|---|---|---|
| A pure kernel holds for every input within stated bounds, with no panic or overflow | Kani | `#[kani::proof]` harnesses under `cfg(kani)` in the crate that owns the kernel | `kani` |
| A kernel holds over traces of any length | Verus, on a twin of the production kernel | `rust/verus`, linked to production by `rust/verus/equivalence` | `verus` |
| A protocol invariant holds over every interleaving | TLA+ checked by TLC | `models/<Name>/` | `models` |
| A store or cache with I/O matches a reference on random inputs | proptest | crate `tests/` | `rust-tests` |
| A TypeScript lifecycle or client holds on random inputs and async interleavings | fast-check | `*.property.test.ts` in the package | `ts-check` |
| A decoder of untrusted bytes never panics | cargo-fuzz | `rust/fuzz/fuzz_targets` | `fuzz` |
| Two implementations agree on every case | Generated differential vectors | `fixtures/**/generated-v1.json` from `scripts/vector-generators.toml` | `vectors` |
| The router a product runs has the right layers | Conformance helpers | `baukit-test`, such as `assert_auth_router_conformance` and `assert_ops_router_conformance` | `rust-tests`, fixture steps |
| A code pattern never appears | Static rules | `lint/ast-grep/rules`, run by `baukit check` | `baukit-check` |

Kernels that Kani or Verus check take every input as an argument: no clock
read, no I/O, no lock. Crates keep them in small pure modules, for example
`baukit-ratelimit/src/kernels/` and `baukit-jobs/src/lease.rs`, and the
production code calls them.

Verus runs on twins in `rust/verus`, outside the main workspace, so its
pre-release proc macros stay out of `rust/Cargo.lock`. Each twin uses the
production integer types. Proptests and Kani harnesses in
`rust/verus/equivalence` tie each twin to its production kernel.

A model's `<Name>.cfg` describes the fixed design and must pass. A
`<Name>*_current.cfg` reproduces a bug in the old code and must still report a
violation, which shows the model can see that bug.

A proof only shows the code matches its spec, and Kani only within its
harness bounds. Each harness states its bounds in a `Bounds:` line of its doc
comment. A proven kernel still needs a test through the real wiring.

The current list of harnesses, twins and models with their bounds is in
[verification targets](../verification.md) and [`models/README.md`](../../models/README.md).

## Enforced by

- The `kani` step runs `scripts/list-proofs.py --check`, which fails when a
  harness has no `Bounds:` line. It then runs `cargo kani` on every crate with
  `cfg(kani)` code and fails when a crate verifies fewer harnesses than it
  lists.
- The `verus` step runs `cargo verus verify` on `rust/verus`, the equivalence
  proptests and the equivalence Kani harnesses.
- The `models` step runs `models/check.sh`, which fails when a fixed config
  finds a violation or a `_current` config finds none.
- The `vectors` step runs `scripts/regenerate-vectors.py --check`.
- The `fuzz` step runs every target for 60 seconds through `scripts/fuzz.sh`.

Nothing checks that a new kernel, protocol or decoder gets the tool its class
calls for. Review catches that, or nobody does.

## Consequences

Maintainers know which tool to reach for and what it proves. Formal steps add
pinned toolchains (Kani, Verus, a TLA+ jar from `deploy/tools.toml`) and run
time, so the gate selects them by changed paths. A twin can drift from
production in ways the equivalence tests miss, and a model can drift from the
code it abstracts. Creusot and Aeneas are not used: they cover the same ground
as Verus here with less tooling.

## History

- 2026-11-07, `09a6847c`: fuzz targets and property tests added.
- 2026-11-25, `44eba6a4`: the tool-per-property choice was written down in the
  correctness and verification plan.
- 2026-11-25, `7762749e`: TLA+ models M-01 to M-08 and `models/check.sh` added.
- 2026-11-25, `3768457c`: Verus twins of the HLC and token bucket added.
- 2026-11-25, `fe8d215f`: `kani`, `models`, `verus`, `migrations`, `workflows`
  and `vectors` became gate steps.
