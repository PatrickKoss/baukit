# Verification targets

Each target runs one `scripts/ci-gate.sh` step, the same command its CI job runs. Tools come from
`mise.toml` (`make toolchain`); the TLA+ jar comes from `deploy/tools.toml` and can be overridden
with `TLA2TOOLS_JAR`.

| Target | Step | What it proves |
|---|---|---|
| `make kani` | `kani` | Every `#[kani::proof]` in a crate with `cfg(kani)` code holds for all inputs within the bounds its doc comment states. Also fails when a harness has no `Bounds:` line. |
| `make kani-full` | `kani-full` | The same for every crate at once. Runs nightly. |
| `make models` | `models` | TLC finds no invariant violation or unintended deadlock in any `models/*/*.cfg`. A cfg opts out of deadlock checking with `\* tlc-args: -deadlock`. |
| `make fuzz` | `fuzz` | No fuzz target crashes within 60 s. |
| `make fuzz-long` | `fuzz-nightly` | The same for `FUZZ_SECONDS` per target (default 1800), growing the corpus. |
| `make migrations-check` | `migrations` | New migrations pass squawk, cross-crate order matches `rust/migrations.toml`, every new file reruns cleanly, and the previous release's store tests pass on HEAD's schema. |
| `make workflows-audit` | `workflows` | actionlint and zizmor (auditor persona) report nothing of low severity or above, and every OIDC job is gated and publish-only. |
| `make vectors` | `vectors-regenerate` | Rewrites every `fixtures/**/generated-v1.json` from `scripts/vector-generators.toml`. |
| `make vectors-check` | `vectors` | The committed vectors match what the generators produce. |
| `make proofs-list` | none | Prints the Kani harness table with each harness's bounds. |
| `make verify` | kani, models, vectors, migrations, workflows | The pre-push set. `make ci` includes it. |

A proof covers only what its harness or model states. Read the bounds column before relying on one.
