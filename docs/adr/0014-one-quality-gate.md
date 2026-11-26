# ADR 0014: One gate script runs every CI check locally, in parallel lanes

## Status
Accepted, 2026-11-27.

## Context

CI used to run its own commands per job, and local checks were a different list. A change could pass
locally and fail in CI, or the other way round. `cargo test` in CI also skipped the Docker-gated
tests, so the integration tests ran nowhere.

## Decision

- `scripts/ci-gate.sh` holds every check as a named step and runs them in parallel lanes through
  `gate.py`, the same runner strict products get from `templates/common/__strict__/`.
  `--list-steps` prints the table, `--step NAME` runs one, `--affected BASE` runs the lanes mapped
  to changed paths, and `--plan` prints the selection.
- Each job in `.github/workflows/ci.yml` calls `scripts/ci-gate.sh --step NAME`, so CI and a local
  run use the same commands. Rust tests run with `--include-ignored`.
- `scripts/ci-gate.toml` maps paths to lanes under `[quality.affected]`. A change to a `shared` path
  selects everything. It sets `gate_budget_minutes` to 30 for a full run and 15 for an affected
  run, and the runner warns when a run goes over. It also sets disk floors.
- `platform-e2e` is its own lane, selected only by deploy paths. `deploy/platform/e2e/run.sh` takes
  a lock under `$XDG_CACHE_HOME/baukit`, and with `BAUKIT_HEAVY_SLOT_DIR` set it waits for a free
  heavy-run slot.
- Products choose `--quality standard` (the default) or `--quality strict`. Strict adds
  `scripts/quality-gate.sh` and a blocking CI job. See the
  [quality gates guide](../guide/quality-gates.md).

## Enforced by

- `scripts/test_ci_gate.py`: `test_every_step_function_exists_for_each_listed_step`,
  `test_platform_e2e_is_its_own_selected_lane`,
  `test_platform_e2e_runs_for_deploy_changes_but_not_for_rust_changes` and
  `test_ci_matrix_covers_the_same_flavors`.
- `templates/common/__strict__/scripts/gate.test.py`: `test_over_budget_warns_with_the_slowest_lane` and `test_budget_defaults`.
- Nothing fails a run that goes over budget. The runner only warns.

## Consequences

A green local gate predicts a green pipeline. A full run is heavy, so the affected mode and single
steps are the normal loop while iterating. Standard products have no gate script and run their CI
steps one by one.

## History

- 2026-11-02, commits `ec5488aa` and `b611d161`: CI jobs moved onto `ci-gate.sh` steps, and Rust
  tests started running with `--include-ignored`.
- 2026-11-19, commit `9e4171e1`: worker caps in `baukit.toml`.
