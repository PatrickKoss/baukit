# Quality gate

`sh scripts/quality-gate.sh` runs the strict gate. `make ci` runs the same script and nothing else. `scripts/gate.py` is the runner. It reads the step table from `sh scripts/quality-gate.sh --list-steps`, starts one thread per lane, and calls `sh scripts/quality-gate.sh --step NAME` for each step.

## Lanes

| Lane | Steps in order |
| --- | --- |
| fast | openapi, rust-fmt, migrations, mcp-drift, scripts-tests, markdown-links, preflight, metric-names, spec-purposes |
| backend | rust-clippy, rust-msrv, rust-tests, rust-doctests |
| images | images |
| web | web-install, web-build, web-lint, web-unit, web-service-worker, {% if context.auth_enabled and not context.auth_oidc %}web-bundle, {% endif %}playwright, webkit-repeats |
| mobile | mobile-install, mobile-typecheck, mobile-lint, expo-doctor, mobile-unit, mobile-service-worker, {% if context.auth_enabled and not context.auth_oidc %}mobile-bundle, {% endif %}ios-check, android-build, android-e2e |
| stack | full-stack-e2e, only when `quality.full_stack_e2e = true` |

A product only gets the lanes and steps its capabilities need. Lanes run at the same time. Steps inside a lane run in order and stop at the first failure. Two steps also wait on a step in another lane: `web-build` and `mobile-typecheck` wait for `openapi`, because the OpenAPI step regenerates the client those builds import. `mcp-drift` waits for `openapi` too.

Each lane writes `.quality-gate/<lane>.log`. Every step appends `name seconds` to `.quality-gate/timings.txt`, which also gets `lane:<lane> seconds` and `total seconds` lines. The gate prints the step table slowest first at the end. When a lane fails, the gate prints the last 40 lines of its log, lists every failed lane, and exits 1. Set `BAUKIT_GATE_LOG_DIR` to write the logs elsewhere. `sh scripts/quality-gate.sh --plan` prints the chosen steps and stops.

## Workers

`BAUKIT_GATE_CORES` sets the cores the gate may use. It defaults to `nproc`. All lanes draw from that one budget. When a step starts, the runner divides the budget by the lanes that still have steps to run, not counting light lanes, and caps the result so each worker has `BAUKIT_GATE_WORKER_MEMORY_MB` (default 1024) of the host's total memory. The step takes that many cores out of the budget, or what is left, and returns them when it ends. A step waits while the budget is empty, so the running steps never hold more than `BAUKIT_GATE_CORES` together, and a lane that finishes early leaves bigger shares to the steps that start after it. The share goes to nextest `--test-threads`, `CARGO_BUILD_JOBS`, Jest `--maxWorkers`, `VITEST_MAX_WORKERS`, Playwright `E2E_WORKERS`, Gradle `--max-workers` (`BAUKIT_QA_GRADLE_WORKERS`) and the CMake job pools of native modules (`BAUKIT_QA_NATIVE_JOBS`, applied by `mobile/scripts/qa/native-jobs.init.gradle`). The gate never reads `Committed_AS`.

`android-e2e` waits until it can take at least `BAUKIT_GATE_EMULATOR_CORES` cores, so the emulators keep their share while other lanes still run. It defaults to 2 per emulator, from `quality.android_emulators` or `BAUKIT_QA_ANDROID_EMULATORS`, and never more than the budget.

`quality.light` in `baukit.toml` lists lanes or steps that take no share, for steps that mostly wait or run one process. Its default is `[]`. The `fast` lane is always light. A light step still gets the current share in its environment. The gate and `baukit doctor` reject names that are no lane or step of the gate. Each step prints the cores it started with, and the lane log records them after the step name.

Tests that must not run in parallel belong in a nextest test group in `backend/.config/nextest.toml` or in a separate Jest project. Don't add `--runInBand` or a one-thread setting for the whole suite.

## Single runs

Each suite runs once. nextest runs every Rust test, including ignored ones, under `cargo llvm-cov`, and the coverage floor comes from that run. `rust-doctests` runs `cargo test --doc` because nextest can't run doctests. Jest and Vitest coverage come from the only unit test run. `android-build` builds the debug and release APKs from one prebuild, and `android-e2e` installs that release APK for Maestro.

## Caches

The gate prints its cache settings before it starts. It sets `RUSTC_WRAPPER=sccache` when `sccache` is on `PATH` and prints `sccache on` or `sccache off`. `CARGO_HOME` defaults to `~/.cargo`, `GRADLE_USER_HOME` to `~/.gradle` and `PLAYWRIGHT_BROWSERS_PATH` to `~/.cache/ms-playwright`, so worktrees share downloads. Cargo, Gradle, pnpm and Playwright lock their caches, so two worktrees can gate at once. `backend/Dockerfile` mounts the cargo registry and git directories with `sharing=locked`, and keeps the target directory per checkout through `CARGO_TARGET_CACHE_PREFIX`.

## Android emulator

`quality.android_emulator_port` in `baukit.toml` sets the emulator console port. `BAUKIT_QA_ANDROID_PORT` overrides it for one run. `quality.android_emulators`, 2 by default, sets how many emulators the gate boots. Each emulator takes its console port and the ADB port after it, and the next emulator starts two ports higher, so two emulators at 5644 take 5644 to 5647. `BAUKIT_QA_ANDROID_EMULATORS` overrides the count for one run.

Ports from 5556 to 5811 form blocks of 8, starting at 5556, 5564, 5572 and so on. A block holds up to 4 emulators. Each product owns one block, and its range must stay inside the block that holds its first port. `baukit doctor` prints the range and the block, and rejects an odd port, a port outside 5556 to 5810, or a range that runs past its block. `baukit new` picks one of the 16 blocks from 5556 to 5683 from the product name. The blocks from 5684 up stay free for Baukit's own lanes and one-off runs: Baukit's `native-android` lane uses 5780 to 5783, from `scripts/ci-gate.toml`. Two products in one block share emulators, so give each product its own block.

The products' ranges at Baukit v0.17.1, from each `baukit.toml`:

| Product | Port | Emulators | Range | Block | Overlaps or past its block |
| --- | --- | --- | --- | --- | --- |
| Eigenruhe | 5564 | 3 | 5564 to 5569 | 5564 to 5571 | |
| Hebkit | 5592 | 2 | 5592 to 5595 | 5588 to 5595 | Kassensturz |
| Kassensturz | 5594 | 2 | 5594 to 5597 | 5588 to 5595 | Hebkit, past its block |
| Schlauzug | 5598 (script default, not in `baukit.toml`) | 2 | 5598 to 5601 | 5596 to 5603 | |
| Redemut | 5620 | 2 | 5620 to 5623 | 5620 to 5627 | Baukit's old native lane |
| Schlüsselbund | 5644 | 2 | 5644 to 5647 | 5644 to 5651 | Tiefgang |
| Tiefgang | 5646 | 2 | 5646 to 5649 | 5644 to 5651 | Schlüsselbund |
| Solo Leveling System | 5662 | 2 | 5662 to 5665 | 5660 to 5667 | |
| Lesezeichen | 5674 | 2 | 5674 to 5677 | 5668 to 5675 | past its block |
| Leitbild | 5680 | 2 | 5680 to 5683 | 5676 to 5683 | |

Architecture Health Platform and Runtime Analyzer have no mobile app. A product that moves picks a free block, sets the block's first port, and keeps the second free block for itself only. The free product blocks start at 5556, 5572, 5580, 5604, 5612, 5628, 5636 and 5652.

The first boot is a cold boot. Once systemui is idle and the health check passes, `android-env.sh` runs `cmd package bg-dexopt-job` and then reads the guest's `/proc/stat` every 10 seconds until the emulator uses less than one core over a window. `BAUKIT_QA_SETTLE_TIMEOUT_SECONDS`, 600 by default, bounds the dexopt run and the wait together. Past it, the script prints `did not settle` and saves anyway. A device that drops off ADB while it settles fails the start. The script then repeats the health check, clears logcat and saves the snapshot, so later boots don't resume the first boot's dexopt and Play services work.

The snapshot is named `baukit-clean-<hash>`. The hash covers the system image path and its `source.properties`, the `hw.*` settings in the AVD's `config.ini`, the GPU mode and the Baukit template version, so a change to any of them makes a new snapshot instead of loading a stale one. After saving a new snapshot, the script deletes the AVD's other `baukit-clean*` snapshots, about a gigabyte each. It marks a finished save with `<avd>.avd/<snapshot>.ready` and loads only a marked snapshot. The cold boot and save hold a lock on `<avd>.avd/baukit-snapshot.lock`, so a second gate of the same product waits for the first save and then loads it. Delete the `.ready` file to force a cold boot. The emulators render with `-gpu swiftshader_indirect`. After `sys.boot_completed`, the script waits until systemui has a stable PID and a focused window that is no "Not Responding" dialog. The health check reads `am_anr` and `am_crash` lines from logcat and `dumpsys activity lastanr`, and fails on either. The first emulator boots alone and saves the snapshot if it's missing. The others then boot from that snapshot with `-read-only`, and each gets the same idle and health checks and the release APK. With two or more emulators, `run-maestro.sh` passes every flow once to `maestro test --shard-split <n>` with all serials in `--device`, and it checks every device while Maestro runs. When one disappears it stops Maestro with `emulator <serial> died during the sharded run`. With one emulator, or one flow, the flows run in order and the script checks the device before each one, stopping with `emulator <serial> died before <flow>`. Flows don't rerun.

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

The gate prints the mode, the reason for each changed file and the step list before it runs anything. `baukit doctor` rejects globs with `..` and absolute globs. It runs `sh scripts/quality-gate.sh --list-steps` and rejects names that are no lane, step or token in that listing, so a step the product adds to its gate can be named here.

## Budget

`quality.gate_budget_minutes` holds the target wall time for the full and the affected gate. New products start at 10 and 5 minutes, the small tier. Mid-size repositories set 20 and 10, large ones 30 and 15. At the end the gate prints whether it stayed within the budget. Over budget, it prints a warning with the lane that set the wall time and that lane's steps, slowest first. The gate doesn't fail over budget.
