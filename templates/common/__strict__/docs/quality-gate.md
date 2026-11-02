# Quality gate

`sh scripts/quality-gate.sh` runs the strict gate. `make ci` runs the same script and nothing else. `scripts/gate.py` is the runner. It reads the step table from `sh scripts/quality-gate.sh --list-steps`, starts one thread per lane, and calls `sh scripts/quality-gate.sh --step NAME` for each step.

## Lanes

| Lane | Steps in order |
| --- | --- |
| fast | openapi, rust-fmt, migrations, mcp-drift, scripts-tests, markdown-links, preflight, metric-names, spec-purposes |
| backend | rust-clippy, rust-msrv, rust-tests, rust-doctests |
| images | images |
| web | web-install, web-build, web-lint, web-unit, web-service-worker, playwright, webkit-repeats |
| mobile | mobile-install, mobile-typecheck, mobile-lint, expo-doctor, mobile-unit, mobile-service-worker, ios-check, android-build, android-e2e |
| stack | full-stack-e2e, only when `quality.full_stack_e2e = true` |

A product only gets the lanes and steps its capabilities need. Lanes run at the same time. Steps inside a lane run in order and stop at the first failure. Two steps also wait on a step in another lane: `web-build` and `mobile-typecheck` wait for `openapi`, because the OpenAPI step regenerates the client those builds import. `mcp-drift` waits for `openapi` too.

Each lane writes `.quality-gate/<lane>.log`. Every step appends `name seconds` to `.quality-gate/timings.txt`, which also gets `lane:<lane> seconds` and `total seconds` lines. The gate prints the step table slowest first at the end. When a lane fails, the gate prints the last 40 lines of its log, lists every failed lane, and exits 1. Set `BAUKIT_GATE_LOG_DIR` to write the logs elsewhere. `sh scripts/quality-gate.sh --plan` prints the chosen steps and stops.

## Workers

`BAUKIT_GATE_CORES` sets the cores the gate may use. It defaults to `nproc`. The runner divides it by the number of lanes other than `fast` that have steps to run, and caps the result so each worker has `BAUKIT_GATE_WORKER_MEMORY_MB` (default 1024) of the host's total memory. The share goes to nextest `--test-threads`, `CARGO_BUILD_JOBS`, Jest `--maxWorkers`, `VITEST_MAX_WORKERS`, Playwright `E2E_WORKERS` and Gradle `--max-workers`. The gate never reads `Committed_AS`.

Tests that must not run in parallel belong in a nextest test group in `backend/.config/nextest.toml` or in a separate Jest project. Don't add `--runInBand` or a one-thread setting for the whole suite.

## Single runs

Each suite runs once. nextest runs every Rust test, including ignored ones, under `cargo llvm-cov`, and the coverage floor comes from that run. `rust-doctests` runs `cargo test --doc` because nextest can't run doctests. Jest and Vitest coverage come from the only unit test run. `android-build` builds the debug and release APKs from one prebuild, and `android-e2e` installs that release APK for Maestro.

## Caches

The gate prints its cache settings before it starts. It sets `RUSTC_WRAPPER=sccache` when `sccache` is on `PATH` and prints `sccache on` or `sccache off`. `CARGO_HOME` defaults to `~/.cargo`, `GRADLE_USER_HOME` to `~/.gradle` and `PLAYWRIGHT_BROWSERS_PATH` to `~/.cache/ms-playwright`, so worktrees share downloads. Cargo, Gradle, pnpm and Playwright lock their caches, so two worktrees can gate at once. `backend/Dockerfile` mounts the cargo registry and git directories with `sharing=locked`, and keeps the target directory per checkout through `CARGO_TARGET_CACHE_PREFIX`.

## Android emulator

`quality.android_emulator_port` in `baukit.toml` sets the emulator console port. `baukit new` writes an even port from 5556 to 5680 derived from the product name, so products on one host don't share an emulator. `BAUKIT_QA_ANDROID_PORT` overrides it for one run. `quality.android_emulators`, 2 by default, sets how many emulators the gate boots. They take the console port and the even ports after it, so two emulators at 5644 use 5644 and 5646. `BAUKIT_QA_ANDROID_EMULATORS` overrides the count for one run.

The first boot is a cold boot. Once systemui is idle and the health check passes, `android-env.sh` saves a `baukit-clean` snapshot, and later boots load it. Delete `<avd>.avd/snapshots/baukit-clean` to force a cold boot. After `sys.boot_completed`, the script waits until systemui has a stable PID and a focused window that is no "Not Responding" dialog. The health check reads `am_anr` and `am_crash` lines from logcat and `dumpsys activity lastanr`, and fails on either. The first emulator boots alone and saves the snapshot if it's missing. The others then boot from that snapshot with `-read-only`, and each gets the same idle and health checks and the release APK. With two or more emulators, `run-maestro.sh` passes every flow once to `maestro test --shard-split <n>` with all serials in `--device`, and it checks every device while Maestro runs. When one disappears it stops Maestro with `emulator <serial> died during the sharded run`. With one emulator, or one flow, the flows run in order and the script checks the device before each one, stopping with `emulator <serial> died before <flow>`. Flows don't rerun.

## Browser matrix

Tag a Playwright test with its purpose:

```ts
test('totals add up', { tag: '@logic' }, async ({ page }) => { /* ... */ });
test('sticky header stays visible', { tag: '@browser' }, async ({ page }) => { /* ... */ });
```

`@logic` tests check application behaviour and run on `desktop-chromium` only. `@browser` tests and untagged tests run on all four projects. The protected `qa-*.spec.ts` specs always run on all four projects, and the `spec-purposes` step fails when one of them carries `@logic`. Specs in `quality.critical_paths` run on both WebKit projects `quality.webkit_repeats` times with every purpose included, and the main WebKit pass skips them.

## Affected gate

`sh scripts/quality-gate.sh --affected <base>` compares the merge base of `<base>` and `HEAD` with the working tree, including untracked files, and runs a subset:

1. A changed file that matches a shared path runs the full gate.
2. A changed file that matches no glob in `quality.affected` runs the full gate.
3. Otherwise every matching glob adds its lanes or steps. With no changes, only the fast checks run.

The fast checks always run: the whole `fast` lane plus format, lint and typecheck steps in the other lanes. A lane name selects every step in that lane. A step name selects that step. `web-e2e` selects the web build and Playwright, and `playwright:@tag` runs only Playwright tests with that tag. Selected Rust tests run with nextest for the changed crates and the crates that depend on them, found with `cargo metadata`. Selected Playwright runs use `--only-changed=<base>` unless a tag was chosen. Jest uses `--changedSince <base>` and Vitest uses `--changed <base>`. Android and Maestro run only when the mobile lane is selected. The affected gate skips coverage floors. The full gate still enforces them.

The shared paths are lockfiles, `package.json` files, `pnpm-workspace.yaml`, Rust toolchain files, `.cargo/`, every `migrations/` directory, `baukit.toml`, `limits.json`, `scripts/`, `Makefile`, `mise.toml`, `.github/`, `compose.yaml`, the OpenAPI document, `mcp-tools.json` next to it, every `openapi.consumers` file and every glob in `quality.shared`.

The gate prints the mode, the reason for each changed file and the step list before it runs anything. `baukit doctor` rejects globs with `..`, absolute globs and names that are no lane or step of this product.

## Budget

`quality.gate_budget_minutes` holds the target wall time for the full and the affected gate. New products start at 10 and 5 minutes, the small tier. Mid-size repositories set 20 and 10, large ones 30 and 15. At the end the gate prints whether it stayed within the budget. Over budget, it prints a warning with the lane that set the wall time and that lane's steps, slowest first. The gate doesn't fail over budget.
