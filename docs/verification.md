# Verification targets

Each target runs one `scripts/ci-gate.sh` step, the same command its CI job runs. Tools come from
`mise.toml` (`make toolchain`); the TLA+ jar comes from `deploy/tools.toml` and can be overridden
with `TLA2TOOLS_JAR`.

| Target | Step | What it proves |
|---|---|---|
| `make kani` | `kani` | Every `#[kani::proof]` in a crate with `cfg(kani)` code holds for all inputs within the bounds its doc comment states. Also fails when a harness has no `Bounds:` line. |
| `make kani-full` | `kani-full` | The same for every crate at once. Runs nightly. |
| `make verus` | `verus` | The Verus proofs in `rust/verus` (V-01 to V-03, below), the proptests that compare each twin with production, and the Kani harnesses that prove the HLC and bucket twins equal to production. |
| `make models` | `models` | `models/check.sh`: each `models/<Name>/<Name>.cfg` passes TLC (deadlock checking off), and each `<Name>*_current.cfg`, which reproduces a bug in today's code, reports a violation. |
| `make fuzz` | `fuzz` | No fuzz target crashes within 60 s. |
| `make fuzz-long` | `fuzz-nightly` | The same for `FUZZ_SECONDS` per target (default 1800), growing the corpus. |
| `make migrations-check` | `migrations` | New migrations pass squawk, cross-crate order matches `rust/migrations.toml`, every new file reruns cleanly, and the previous release's store tests pass on HEAD's schema. |
| `make workflows-audit` | `workflows` | actionlint and zizmor (auditor persona) report nothing of low severity or above, and every OIDC job is gated and publish-only. |
| `make vectors` | `vectors-regenerate` | Rewrites every `fixtures/**/generated-v1.json` from `scripts/vector-generators.toml`. |
| `make vectors-check` | `vectors` | The committed vectors match what the generators produce. |
| `make proofs-list` | none | Prints the Kani harness table with each harness's bounds. |
| `make verify` | kani, models, vectors, migrations, workflows | The pre-push set. `make ci` includes it. |

A proof covers only what its harness or model states. Read the bounds column before relying on one.

## Verus

`rust/verus` holds verified twins of three production kernels. Verus proves each theorem for
traces of any length, where Kani stops at its unwind bound.

| ID | Twin | Production | Proven |
|---|---|---|---|
| V-01 | `hlc.rs` `replay` | `baukit_sync::hlc` `now_step`, `observe_step`, `HybridLogicalClock::{now, observe}` | Timestamps strictly increase and exceed the starting state. The final `wall * 1000 + counter` is at most `max(start, (max_physical + max_drift) * 1000 + 999)` plus the number of calls, so the wall clock passes `max_physical + max_drift` only by counter roll-over, 1 ms per 1000 calls. |
| V-02 | `bucket.rs` `replay` | `baukit_ratelimit::kernels::bucket` `refill`, `admit`, and the entry update in `InMemoryRateLimitStore::check_at` | For one key with a fixed scaled quota, starting full at `t0`: admitted cost is at most `capacity + rate * (t - t0)`, `t` the latest request time. Out-of-order times count as no refill, as in production. |
| V-03 | `pull.rs` `page_progress_and_no_skip` | `baukit_sync::pull::merge_pull_page` | Given table reads that honour `(revision, id) > after`, `revision <= hi` and `LIMIT limit` with `limit >= 1`: the page sets `more` and moves `next` strictly past `after`, or clears `more` and returns `after_revision(hi)`. Every window row is in the page or above `next`. |

The twins use the production integer types and the same checked and saturating arithmetic,
written as explicit range checks Verus can reason about. Two links keep them honest:
`rust/verus/equivalence` runs proptests over random and edge-case inputs for all three twins, and
Kani harnesses that prove twin and production equal: HLC `now_step`, `observe_step`, `encode` and
`decode` over unconstrained `i64`; bucket `refill` and `admit` at rates 0, 1, 7 and `u64::MAX`
with the other inputs `<= 15` or `u64::MAX`. The pull merge has no Kani harness: CBMC needs more
than 8 GB on the production `Uuid` byte comparison and sort even for two one-row tables, so its
link is the proptest (4096 cases).

### Layout

The plan offered two layouts. (a) puts the production kernels inside `verus!` and depends on
`vstd`. vstd's erased build compiles on MSRV 1.95 and passes `cargo deny`, so (a) was possible.
I chose (b), a twin outside the main workspace, because (a) adds a `0.0.0` pre-release proc macro
(`verus_builtin_macros` on a forked `syn`) to the build of every product that uses `baukit-sync`
or `baukit-ratelimit`, and moves weekly with each Verus release. The twin has its own
`[workspace]` and `Cargo.lock`, so none of that reaches `rust/Cargo.lock`.

### Running it

`make verus` runs the gate step. Verus comes from `mise.toml` (`http:verus`, release zip pinned
by sha256, z3 bundled inside it) and needs the pinned rust 1.100.0 toolchain. For the proofs alone:

```sh
cargo verus verify --manifest-path rust/verus/Cargo.toml
```

A Verus release bump changes the `vstd` pin in `rust/verus/Cargo.toml`; `cargo verus new` prints
the matching one.
