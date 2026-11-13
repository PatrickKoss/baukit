# Quality gate

`sh scripts/quality-gate.sh` runs the strict gate. `make ci` runs the same script and nothing else. `scripts/gate.py` is the runner. It reads the step table from `sh scripts/quality-gate.sh --list-steps`, starts one thread per lane, and calls `sh scripts/quality-gate.sh --step NAME` for each step.

## Lanes

| Lane | Steps in order |
| --- | --- |
| fast | openapi, rust-fmt, migrations, mcp-drift, scripts-tests, markdown-links, preflight, baukit-doctor, baukit-check, workflow-lint, secret-scan, osv, {% if context.backend %}cargo-deny, {% endif %}python-lint, shell-lint, metric-names, spec-purposes |
| backend | rust-clippy, rust-msrv, rust-tests, rust-doctests |
{% if context.backend %}| fuzz | fuzz |
{% endif %}| images | images |
| web | web-install, web-build, web-lint, web-unit, web-service-worker, {% if context.auth_enabled and not context.auth_oidc %}web-bundle, {% endif %}playwright, webkit-repeats |
| mobile | mobile-install, mobile-typecheck, mobile-lint, expo-doctor, mobile-unit, mobile-service-worker, {% if context.auth_enabled and not context.auth_oidc %}mobile-bundle, {% endif %}ios-check, android-build, android-e2e |
| stack | full-stack-e2e, only when `quality.full_stack_e2e = true` |

A product only gets the lanes and steps its capabilities need. Lanes run at the same time. Steps inside a lane run in order and stop at the first failure. Two steps also wait on a step in another lane: `web-build` and `mobile-typecheck` wait for `openapi`, because the OpenAPI step regenerates the client those builds import. `mcp-drift` waits for `openapi` too.

Each lane writes `.quality-gate/<lane>.log`. Every step appends `name seconds` to `.quality-gate/timings.txt`, which also gets `lane:<lane> seconds` and `total seconds` lines. The gate prints the step table slowest first at the end. When a lane fails, the gate prints the last 40 lines of its log, lists every failed lane, and exits 1. Set `BAUKIT_GATE_LOG_DIR` to write the logs elsewhere. `sh scripts/quality-gate.sh --plan` prints the chosen steps and stops.

`baukit-check` runs `baukit check agent-docs file-size suppressions lints patterns observability template-files` from the same Baukit checkout as `baukit-doctor`, with the limits in `baukit-check.toml`. `lints` compares `[workspace.lints]` in `backend/Cargo.toml` with the Baukit baseline, `patterns` runs the Baukit ast-grep rules, `observability` checks that dashboards and rules under `deploy/observability` name only metrics the backend emits or the Baukit catalog defines, and `template-files` compares Baukit-owned files with `.baukit/template-files.toml`. The other fast steps call `sh scripts/repo-lint.sh`, which runs actionlint and zizmor on `.github/workflows`, gitleaks, osv-scanner on the committed lockfiles, {% if context.backend %}`cargo deny` with `backend/deny.toml`, {% endif %}ruff on Python files and shellcheck on shell scripts. `mise install` installs the versions pinned in `mise.toml`.
{% if context.backend %}
`fuzz` builds every cargo-fuzz target in `backend/fuzz` with nightly Rust and runs each one for `BAUKIT_FUZZ_SECONDS` (default 60). It runs in the full gate, not in the fast lane. Crashing inputs land in `backend/fuzz/artifacts/<target>/`. Add a target per parser that takes untrusted bytes, next to `item_body`.
{% endif %}

## Product extensions

Baukit owns `scripts/`, `web/scripts/`, `mobile/scripts/`, `.github/workflows/`, this document and a few other files. `.baukit/template-files.toml` lists each one with the SHA-256 of its rendered content, and `baukit check template-files` fails when one of them changes. Don't edit them. Put product steps in `scripts/quality-gate.local.sh` instead. The gate sources that file when it exists. A `local_steps` function prints extra rows for the step table, in the format of `sh scripts/quality-gate.sh --list-steps`, and a `step_<name>` function runs each one, with dashes in the name turned into underscores:

```sh
local_steps() {
  echo "dashboard-coverage fast - always -"
}

step_dashboard_coverage() {
  python3 tools/check-dashboards.py
}
```

The file can also redefine a template step function when a product has to replace it. CI runs the local steps as part of `sh scripts/quality-gate.sh`. A product on the standard profile has no gate, and its generated CI runs each `local_steps` row from the `Generated repository scripts` job instead.

After a Baukit upgrade re-renders the template, review the changes to the owned files and run `baukit check template-files --update` to record the new hashes.

{% if context.mobile %}`mobile/scripts/qa/local.sh` is the product hook file for the QA scripts. `mobile/scripts/qa/local.sh.example` lists the variables a hook can rely on and the hooks themselves; start your file with `# shellcheck shell=bash`. `run-maestro.sh` also reads the hook `qa_maestro_flows index`, which prints the flow files one device runs. `qa_ios_bundle_id` (or `BAUKIT_QA_IOS_BUNDLE_ID`) sets the iOS bundle id, `qa_apk_variant` (or `BAUKIT_QA_APK_VARIANT`) set to `debug` builds a debug APK with the JS bundle embedded, and `BAUKIT_QA_GRADLE_LOG` tees the Gradle output to a file. When `android-env.sh start` or `boot` fails, it stops the emulators it started. `run-maestro.sh` re-applies `adb reverse` before every flow. `services.sh`, `android-env.sh` and `run-maestro.sh` source it when it exists. It can set `qa_emulator_args` (extra `emulator` arguments, for example `-accel off`), `qa_reverse_ports` (extra `adb reverse` ports), `qa_maestro_args` (extra `maestro test` arguments, for example `--exclude-tags slow`){% if context.backend %}, `qa_compose_services` (the compose services to start, all when empty) and `qa_cargo_bins` (extra binaries built with the API). It can also define the functions `qa_services_env` (after the template exports, to add or override environment), `qa_before_api` (after migrations, for example to seed data), `qa_after_api` (once the API is ready, to start a peer service) and `qa_stop_services` (to stop what the other hooks started){% endif %}.

{% endif %}{% if context.web %}`web/src/design-tokens.json` replaces the example tokens in `web/scripts/generate-tokens.mjs`. The script rejects tokens that miss the WCAG contrast checks. `web/src/tokens.extra.css` is appended to the generated `tokens.css`.

{% endif %}
## Workers

`BAUKIT_GATE_CORES` sets the cores the gate may use. It defaults to `nproc`. All lanes draw from that one budget. When a step starts, the runner divides the budget by the lanes that still have steps to run, not counting light lanes, and caps the result so each worker has `BAUKIT_GATE_WORKER_MEMORY_MB` (default 1024) of the host's total memory. The step takes that many cores out of the budget, or what is left, and returns them when it ends. A step waits while the budget is empty, so the running steps never hold more than `BAUKIT_GATE_CORES` together, and a lane that finishes early leaves bigger shares to the steps that start after it. `.quality-gate/step-seconds.txt` keeps the last passed time of every step across runs, so a partial run such as `--affected` leaves the other steps' times in place, while `timings.txt` describes only the latest run. A step with a recorded time takes its fraction of the budget by its time against the steps that have not started (steps without a time count as the average), at most the free cores and never below the even share; steps without history keep the even share. The share goes to nextest `--test-threads`, `CARGO_BUILD_JOBS`, Jest `--maxWorkers`, `VITEST_MAX_WORKERS`, Playwright `E2E_WORKERS`, Gradle `--max-workers` (`BAUKIT_QA_GRADLE_WORKERS`) and the CMake job pools of native modules (`BAUKIT_QA_NATIVE_JOBS`, applied by `mobile/scripts/qa/native-jobs.init.gradle`). The gate never reads `Committed_AS`.

`android-e2e` waits until it can take at least `BAUKIT_GATE_EMULATOR_CORES` cores, so the emulators keep their share while other lanes still run. It defaults to 2 per emulator, from `quality.android_emulators` or `BAUKIT_QA_ANDROID_EMULATORS`, and never more than the budget.

`quality.light` in `baukit.toml` lists lanes or steps that take no share, for steps that mostly wait or run one process. Its default is `[]`. The `fast` lane is always light. A light step still gets the current share in its environment. The gate and `baukit doctor` reject names that are no lane or step of the gate. Each step prints the cores it started with, the seconds since the gate started, how long it waited for cores, and how many cores stayed free, for example `gate: [android] android-e2e started with 6 cores, at +222 s after waiting 0 s, 0 of 12 cores free`. The lane log records the cores after the step name.

## Sharing the host with other gates

Two environment variables let several gates run on one machine at the same time. Both are unset by default, which keeps the behavior above, and CI leaves them unset.

`BAUKIT_GATE_LOCK_DIR` names a directory with one `<resource>.lock` file per host resource. A step that needs resources queues a ticket in `$BAUKIT_GATE_LOCK_DIR/queue` that names each one and whether it wants it exclusively or shared. It then takes all its `flock` files in one non-blocking pass, `android` first, `docker-network` next and any other resource after those by name, and releases them when the step ends. It never holds one lock while it waits for another. It starts the pass only when no earlier live ticket conflicts with it, and two tickets conflict when they name one resource and at least one wants it exclusively. A later step therefore cannot overtake an earlier one, which bounds every wait by the holders ahead. A step that needs two locks cannot starve behind single-lock steps that keep arriving, and an exclusive step cannot starve behind a stream of shared ones, because `flock` alone grants in no particular order. The kernel drops a dead gate's ticket and locks. Waiters poll every 50 ms.

Chromium reports `ERR_NETWORK_CHANGED` when `docker network create` or `docker network rm` runs on the host. Starting and stopping containers does not trigger it. The `docker-network` resource covers this. `android-e2e` takes it exclusively because `services.sh` starts the QA backend with `docker compose up`, and compose creates a project network. `android-boot`, `android-e2e` and `android-debug` take `android`, which covers boot, install and Maestro on every emulator of the host. Steps that run through `scripts/with-loopback-network.sh`, which wraps `playwright` and `webkit-repeats`, take no lock because the namespace isolates them from host network changes. A product step that drives a browser on the host network and does not use that wrapper declares `docker-network` under `[quality.shared_resources]`, so such steps run together and wait only for an exclusive holder. A step that creates or removes Docker networks, such as compose with its own network or Testcontainers `with_network`, declares it under `[quality.resources]`.


```toml
[quality.resources]
android = ["emulator-smoke"]
docker-network = ["compose-e2e"]

[quality.shared_resources]
docker-network = ["host-browser-e2e"]
```

With a lock directory set, steps that hold a resource start only after the `fast` lane has passed. When it failed they print `not run (fast lane failed)` and never queue for a lock. A `fast` step therefore cannot run after such a step, directly or through other steps. The gate then exits with status 2 before it runs anything and names both steps. A step waits for its locks before it draws cores, so a queued step holds none.

`BAUKIT_GATE_CORE_POOL` names a directory of `slot-<n>.lock` files, one per core up to `BAUKIT_GATE_CORES`. Every step draws its cores by taking slots with a non-blocking `flock`, so gates on one host never run more cores than the pool has. Each gate registers in `$BAUKIT_GATE_CORE_POOL/gates`, and the share of a step is computed from the pool size divided by the number of live gates, so two gates at once start with half the pool each instead of the first one taking all of it. A step that finds too few slots polls every 200 ms. The kernel frees the slots when a step ends or the gate dies. Step lines report `waited N s for android.lock` and `N of M pool cores free`.

The gate opens lock and slot files with close-on-exec and starts steps with `close_fds`, so emulators, backends and previews that outlive a step hold none of them.

Tests that must not run in parallel belong in a nextest test group in `backend/.config/nextest.toml` or in a separate Jest project. Don't add `--runInBand` or a one-thread setting for the whole suite.

## Single runs

Each suite runs once. nextest runs every Rust test, including ignored ones, under `cargo llvm-cov`, and the coverage floor comes from that run. `rust-doctests` runs `cargo test --doc` because nextest can't run doctests. Jest and Vitest coverage come from the only unit test run. `android-build` builds the debug and release APKs from one prebuild, and `android-e2e` installs that release APK for Maestro.

## Caches

{% if context.web %}The `playwright` and `webkit-repeats` steps run inside a loopback-only network namespace through `scripts/with-loopback-network.sh`. Chromium aborts module requests with `net::ERR_NETWORK_CHANGED` when another process on the host creates or removes a Docker network, and the namespace hides those changes. The helper needs unprivileged user namespaces and exits 69 when they are unavailable. On Ubuntu 24.04 and later, `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` enables them; `ci.yml` does this before the gate. On a host that cannot enable them, `BAUKIT_E2E_NETWORK_ISOLATION=0` runs the suites on the host network and the flake returns. Only suites that talk to the preview server and stubbed APIs belong behind the helper. Suites that start Docker containers or reach the outside network must not use it.

{% endif %}The gate prints its cache settings before it starts. It sets `RUSTC_WRAPPER=sccache` when `sccache` is on `PATH` and prints `sccache on` or `sccache off`. `CARGO_HOME` defaults to `~/.cargo`, `GRADLE_USER_HOME` to `~/.gradle` and `PLAYWRIGHT_BROWSERS_PATH` to `~/.cache/ms-playwright`, so worktrees share downloads. Cargo, Gradle, pnpm and Playwright lock their caches, so two worktrees can gate at once. `backend/Dockerfile` mounts the cargo registry and git directories with `sharing=locked`, and keeps the target directory per checkout through `CARGO_TARGET_CACHE_PREFIX`.

## Android emulator

`quality.android_emulator_port` in `baukit.toml` sets the emulator console port. `BAUKIT_QA_ANDROID_PORT` overrides it for one run. `quality.android_emulators`, 2 by default, sets how many emulators the gate boots. Each emulator takes its console port and the ADB port after it, and the next emulator starts two ports higher, so two emulators at 5644 take 5644 to 5647. `BAUKIT_QA_ANDROID_EMULATORS` overrides the count for one run.

Ports from 5556 to 5811 form blocks of 8, starting at 5556, 5564, 5572 and so on. A block holds up to 4 emulators. Each product owns one block, and its range must stay inside the block that holds its first port. `baukit doctor` prints the range and the block, and rejects an odd port, a port outside 5556 to 5810, or a range that runs past its block. `baukit new` picks one of the 16 blocks from 5556 to 5683 from the product name. The blocks from 5684 up stay free for Baukit's own lanes and one-off runs: Baukit's `native-android` lane uses 5780 to 5783, from `scripts/ci-gate.toml`. Two products in one block share emulators, so give each product its own block.

The products' ranges at Baukit v0.17.1, from `baukit.toml` on each product's `main`:

| Product | Port | Emulators | Range | Block | Overlaps or past its block |
| --- | --- | --- | --- | --- | --- |
| Eigenruhe | 5564 | 3 | 5564 to 5569 | 5564 to 5571 | |
| Hebkit | 5592 | 2 | 5592 to 5595 | 5588 to 5595 | Kassensturz |
| Kassensturz | 5594 | 2 | 5594 to 5597 | 5588 to 5595 | Hebkit, past its block |
| Schlauzug | 5614 | 2 | 5614 to 5617 | 5612 to 5619 | |
| Redemut | 5620 | 2 | 5620 to 5623 | 5620 to 5627 | Baukit's old native lane |
| Schlüsselbund | 5644 | 2 | 5644 to 5647 | 5644 to 5651 | Tiefgang |
| Tiefgang | 5646 | 2 | 5646 to 5649 | 5644 to 5651 | Schlüsselbund |
| Solo Leveling System | 5662 | 2 | 5662 to 5665 | 5660 to 5667 | |
| Lesezeichen | 5674 | 2 | 5674 to 5677 | 5668 to 5675 | past its block |
| Leitbild | 5680 | 2 | 5680 to 5683 | 5676 to 5683 | |

Architecture Health Platform and Runtime Analyzer have no mobile app. A product that moves picks a free block, sets the block's first port, and keeps the second free block for itself only. The free product blocks start at 5556, 5572, 5580, 5596, 5604, 5628, 5636 and 5652.

The first boot is a cold boot. Once systemui is idle and the health check passes, `android-env.sh` runs `cmd package bg-dexopt-job` and then reads the guest's `/proc/stat` every 10 seconds until the emulator uses less than one core over a window. `BAUKIT_QA_SETTLE_TIMEOUT_SECONDS`, 600 by default, bounds the dexopt run and the wait together. Past it, the script prints `did not settle` and saves anyway. A device that drops off ADB while it settles fails the start. The script then repeats the health check, clears logcat and saves the snapshot, so later boots don't resume the first boot's dexopt and Play services work.

The snapshot is named `baukit-clean-<hash>`. The hash covers the system image path and its `source.properties`, the `hw.*` settings in the AVD's `config.ini`, the GPU mode, the emulator features and the Baukit template version, so a change to any of them makes a new snapshot instead of loading a stale one. After saving a new snapshot, the script deletes the AVD's other `baukit-clean*` snapshots, about a gigabyte each, with two exceptions. It keeps any snapshot that a running emulator of the same AVD loaded, read from that process's `-snapshot` argument, and it keeps the newest older snapshot that has its `.ready` marker. A worktree of the same product on the previous template then still finds its snapshot, and the AVD holds at most two finished snapshots plus the ones in use. It marks a finished save with `<avd>.avd/<snapshot>.ready` and loads only a marked snapshot. The cold boot and save hold a lock on `<avd>.avd/baukit-snapshot.lock`, so a second gate of the same product waits for the first save and then loads it. Delete the `.ready` file to force a cold boot. The emulators render with `-gpu swiftshader_indirect`. Products whose flows toggle Wi-Fi set `quality.android_wifi = true`, or `BAUKIT_QA_ANDROID_WIFI=1` for one run. Netsim otherwise shares one Wi-Fi access point across every emulator on the host, so with the setting on the emulators start with `-feature -WiFiPacketStream`. It is off by default, because some apps, Tiefgang's VPN service among them, lost the adb connection of a second emulator under the flag. With the setting on, after each boot the script turns airplane mode off, enables Wi-Fi and waits until `ip -4 route get 10.0.2.2` leaves through `wlan0`. `BAUKIT_QA_WIFI_TIMEOUT_SECONDS`, 90 by default, bounds that wait. The script prints how long each emulator took. Past the limit, the start fails with `has no Wi-Fi route to 10.0.2.2` and prints the last route and the first lines of `cmd wifi status`. After `sys.boot_completed`, the script waits until systemui has a stable PID and a focused window that is no "Not Responding" dialog. The health check reads `am_anr` and `am_crash` lines from logcat and `dumpsys activity lastanr`, and fails on either. The first emulator boots alone and saves the snapshot if it's missing. The others then boot from that snapshot with `-read-only`, and each gets the same idle and health checks and the release APK. With two or more emulators, `run-maestro.sh` passes every flow once to `maestro test --shard-split <n>` with all serials in `--device`, and it checks every device while Maestro runs. When one disappears it stops Maestro with `emulator <serial> died during the sharded run`. With one emulator, or one flow, the flows run in order and the script checks the device before each one, stopping with `emulator <serial> died before <flow>`. Flows don't rerun. `run-maestro.sh` also stops a Maestro run that makes no progress. It counts new output and any new or changed file under the debug directory as progress. When neither changes for `BAUKIT_QA_MAESTRO_STALL_SECONDS`, 300 by default, it stops the Maestro process group and fails with `<flow> made no progress for <n> s`, exit 124. A sequential run applies the deadline to each flow, a sharded run to the whole run. The debug directory keeps the screenshots and `maestro-output.log`, a copy of Maestro's output.

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

## Related settings

`[ports] keycloak_extra = [8181]` lists loopback Keycloak ports for realms the product reads but does not run, such as the hub's suite realm next to its own product realm. It has no default. Doctor accepts issuers of the form `localhost:<port>/realms/` on `ports.keycloak` and on each listed port, and rejects an empty list, zero, duplicates and ports that collide with another host port.

Doctor renders the template's step table for the declared capabilities and fails when `sh scripts/quality-gate.sh --list-steps` lacks one of its steps, such as `expo-doctor` in a mobile product. Product-only steps may be added freely.

`@baukit/sync-client` gives each sync request a deadline. `requestTimeoutMs` on `SyncTransport` defaults to 30000. Past it, the transport aborts the request and rejects with a retryable `SyncNetworkError`, so the next scheduler run opens a new connection.
