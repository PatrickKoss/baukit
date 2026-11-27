# Changelog

## [Unreleased]

## [0.32.0] - 2026-11-27

{% if context.auth_enabled and context.backend %}- `backend/migrations/0011_identity_erasure_operation_id_guard.sql` replaces `erasure_job_failed()` so a failed identity deletion job whose `operationId` is not a UUID no longer aborts the job's status update. Apply it after `0006_identity_erasure_reonboarding.sql`.
{% endif %}{% if context.backend and (context.auth_enabled or context.worker) %}- `backend/migrations/0012_baukit_jobs_deferrals.sql` to `0015_baukit_jobs_running_key_index.sql` add job deferrals and concurrency keys, validate their checks, and build the running-key index `CONCURRENTLY`. `0015` starts with `-- no-transaction`. Apply them in order after `0011_identity_erasure_operation_id_guard.sql`. See `docs/migrations/job-deferrals-and-concurrency-keys.md` in baukit.
{% endif %}- `web/Dockerfile` builds on `nginx/nginx-unprivileged:1.31.10-alpine`, pinned by digest, which ships libxml2 2.13.9-r3 for CVE-2026-86144.
{% if context.quality_strict and context.backend %}- The `fuzz` step of `scripts/quality-gate.sh` runs up to `BAUKIT_FUZZ_JOBS` targets at once, by default the step's `BAUKIT_GATE_WORKERS` share. Each libFuzzer process uses one core, so before most of the share sat idle. Every target still runs for its full `BAUKIT_FUZZ_SECONDS`. Parallel targets write to their own log under `.quality-gate/fuzz-state/`, and the step prints each log whole when its target ends. A crash no longer stops the remaining targets: the step runs them all, then exits 1 and names every target that crashed. Copy `fuzz_target`, `print_fuzz_log`, `fuzz_worker` and `step_fuzz` from the template.
{% endif %}{% if context.quality_strict %}- `scripts/gate.py` reads `[quality.min_cores]` from `baukit.toml`, a table of step names to core counts. Once such a step's dependencies passed, the gate holds its minimum back from the other heavy steps, the way it holds the emulator reserve, and the step waits until it gets at least that many cores. The minimum is clamped to the core budget, and the hold always leaves one core for the other steps. Under a shared core pool the held slots stay free in the pool. Before, a step that asked after the other lanes could get 1 core: baukit's 19-target `fuzz` step ran on 1 core for 2236 s. Without the key nothing changes. Copy `scripts/gate.py` and `scripts/gate.test.py` from the template.
{% endif %}{% if context.quality_strict %}- `scripts/gate.py` skips a step that passed on the same inputs and prints `cached (passed at <sha>, <age>)`. The inputs are the tracked files `--affected` would send to the step, the gate scripts, `baukit.toml` and the run's `BAUKIT_*` settings. The cache works only on a clean tree, keeps passes for `[quality.cache] max_age_days` (7 by default) in `${XDG_CACHE_HOME:-$HOME/.cache}/baukit/gate-results/`, and never serves the steps in `[quality.cache] never`. It is off in CI, with `--no-cache`, with `BAUKIT_GATE_CACHE=off` and for a path dependency on baukit. Copy `scripts/gate.py`, `scripts/gate.test.py`, `scripts/gate_results.py`, `scripts/gate_results.test.py` and `scripts/quality-gate.sh` from the template, add `python3 scripts/gate_results.test.py` to `step_scripts_tests` in a forked `scripts/quality-gate.sh`, and add `[quality.cache]` with `never = ["secret-scan", "osv"{% if context.backend %}, "cargo-deny", "migrations"{% endif %}]` to `baukit.toml`, plus any product step that reads data from outside the repository. `gate.py` now needs `scripts/gate_results.py` next to it, and `baukit doctor` requires both files.
{% endif %}
## [0.31.0] - 2026-11-26

{% if context.auth_oidc and context.backend %}- `config/local.toml` sets `auth.authorized_parties` to this product's web, mobile and local client IDs. Access tokens from other clients now fail locally even with the backend audience. Set the same list in staging and production.
{% endif %}{% if context.auth_enabled and context.backend %}- `backend/migrations/0010_baukit_auth_browser_session_record_ids.sql` adds `baukit_auth_browser_sessions.record_id` for vault v2 session records. Apply it before deploying the backend built against this baukit release.
{% endif %}- `scripts/repo-lint.sh osv` scans every `pnpm-lock.yaml` and `package-lock.json` in the repository, tracked or untracked but not ignored, except those under a `fixtures/`, `vendor/` or `node_modules/` directory. Before, it scanned only `web/pnpm-lock.yaml` and `mobile/pnpm-lock.yaml`, so a root, `extension/`, `content/` or `e2e/` lockfile never reached osv-scanner. Copy `scripts/repo-lint.sh` and `scripts/repo-lint.test.py` from the template and drop any local patch that added lockfiles to the scan. Then run `sh scripts/repo-lint.sh osv`: a newly scanned lockfile can bring advisories it always had.

## [0.30.6] - 2026-11-26

{% if context.quality_strict %}- `scripts/gate_locks.py` has a third lock mode, creator, which `[quality.creator_resources]` in `baukit.toml` sets with the same shape as `[quality.resources]`. Creators of one resource run together. They wait for shared holders and exclusive holders, and those wait for them. Exclusive holders still exclude everyone. For `docker-network` this lets steps that create or remove Docker networks but drive no browser on the host network run at the same time: compose stacks, Testcontainers networks, and Playwright whose browsers run in a `run-server` container. Host-network browsers under `[quality.shared_resources]` still never overlap them. In hebkit's last full gate `playwright` (622 s), `android-e2e` (1033 s) and `webkit-repeats` ran one after another only because each held `docker-network` exclusively. The template default moves `android-e2e` from exclusive to creator on `docker-network`. It still holds `android` exclusively. A step that a product names for one resource in more than one of `[quality.resources]`, `[quality.shared_resources]` and `[quality.creator_resources]` takes it exclusively, so listing `android-e2e` under `[quality.resources] docker-network` keeps the old behaviour.

- How the creator locks stay correct: every holder takes `docker-network.lock`, exclusive holders exclusively and the others shared, so exclusive holders exclude everyone as before. A shared holder also takes `docker-network.shared.lock` shared, then tries `docker-network.creator.lock` exclusively without blocking and lets it go at once. A creator does the mirror image. The try fails while any holder of the other mode runs. Each side takes its own file before it tries the other's, so when a shared holder and a creator arrive at the same moment, at least one of them sees the other and backs off; both can never pass. Holders of one mode never hold each other's file, so they only collide when two tries hit the same instant, and the loser retries on the next 50 ms poll. Queue tickets carry the mode (`docker-network~creator`), and a step passes the queue only when no earlier waiting ticket conflicts with it, so a stream of creators cannot starve a waiting shared step or the other way round. The kernel drops a dead gate's tickets and locks.

- Migration to creator locks: move every step that creates or removes Docker networks and drives no browser on the host network from `[quality.resources]` to `[quality.creator_resources]`. A step that also drives a host-network browser stays under `[quality.resources]`. Older `scripts/gate.py` copies ignore the new key and the step then locks nothing, so add it in the same change that copies `scripts/gate.py`, `scripts/gate_locks.py` and `scripts/gate_locks.test.py` from the template.

- Creator locks with gates on different pins on one host: an older gate reads a creator ticket as an exclusive ticket for an unknown resource named `docker-network~creator`, which conflicts with none of its own steps, so it does not queue behind a waiting creator and only `flock` orders the two. That is safe for exclusive holders: an older exclusive holder, including an older `android-e2e`, still excludes every newer holder, and a newer exclusive holder excludes every older one. It is not safe for older shared holders. They take only `docker-network.lock` shared, so a newer creator, including the default `android-e2e`, can run next to an older `[quality.shared_resources]` step and break its host-network browser with `ERR_NETWORK_CHANGED`. Re-pin products that use `[quality.shared_resources]` (schlauzug, lesezeichen and redemut at 0.30.5) before or together with the others, or keep their gates off the host while newer gates run.

{% endif %}{% if context.quality_strict %}- Since 0.30.5 `scripts/quality-gate.sh` runs `git clone --quiet` and caches the checkout in `${XDG_CACHE_HOME:-$HOME/.cache}/baukit/checkouts/<tag>`. Product tests that fake the old `git clone` argv must accept `--quiet` and set `XDG_CACHE_HOME` to a temporary directory, or a cached checkout from an earlier run skips the fake clone. lesezeichen did this in 5fed90b9.

{% endif %}- `scripts/repo-lint.sh osv` scans every `Cargo.lock` in the repository, tracked or untracked but not ignored, except those under a `fixtures/` or `vendor/` directory. A lockfile with a `Cargo.toml` beside it goes through `scripts/osv-resolved-lock.py` first. Before, the step scanned only `backend/Cargo.lock`, so osv-scanner never saw a second workspace such as laufbefund's `agent/Cargo.lock`. `scripts/repo-lint.sh deny` runs `cargo deny check` on `backend/` and on every directory with a `Cargo.toml` next to such a lockfile, with that directory's `deny.toml` or else `backend/deny.toml`. It checks all of them, names the ones that failed and then exits 1. `scripts/repo-lint.test.py` covers both steps. To adopt it, copy `scripts/repo-lint.sh` and `scripts/repo-lint.test.py`, add `python3 scripts/repo-lint.test.py` to the `generated-scripts` job in `.github/workflows/ci.yml`{% if context.quality_strict %} and to `step_scripts_tests` in `scripts/quality-gate.sh`{% endif %}, and run `baukit check template-files --update`. Then run `sh scripts/repo-lint.sh osv` and `sh scripts/repo-lint.sh deny`. An extra workspace can bring advisories its lockfile always had, or break the license and ban rules of `backend/deny.toml`: update the dependency, add an `[[IgnoredVulns]]` entry with a reason and an `ignoreUntil` date to `osv-scanner.toml`, or give the workspace its own `deny.toml`.{% if context.quality_strict %} A product that replaced `step_cargo_deny` with a direct `cargo deny` call, or that checks extra workspaces in `scripts/quality-gate.local.sh`, switches back to `sh scripts/repo-lint.sh deny` and drops the duplicate. A vendored crate that the product tests on its own keeps its product step.{% endif %}

## [0.30.5] - 2026-11-26

{% if context.quality_strict %}- `scripts/quality-gate.sh` keeps the baukit checkout it needs for `baukit-doctor`, `baukit-check`, `deploy-values`, `metric-names` and the OpenAPI compatibility check in `${XDG_CACHE_HOME:-$HOME/.cache}/baukit/checkouts/v<version>` and reuses it without network on later gates. Before, every gate cloned the tag from GitHub into a temporary directory, so a short GitHub outage failed the whole gate with git exit 128. A missing checkout is cloned into a temporary directory beside the cache path and renamed into place, so concurrent gates never see a partial clone. A failed clone is retried once after 5 seconds, then the gate exits 2 with a message that names the remote and the cache path. `baukit-doctor` and `baukit-check` build the baukit CLI into `.quality-gate/baukit-target` (or `$BAUKIT_GATE_LOG_DIR/baukit-target`), so the cached checkout stays unchanged. Copy `scripts/quality-gate.sh` from the template, or copy `clone_baukit_tag`, `move_into_place`, `cached_baukit_checkout`, `use_baukit_root`, `run_baukit_cli`, `step_baukit_doctor` and `step_baukit_check`.

{% endif %}## [0.30.4] - 2026-11-25

{% if context.mobile %}- The Android idle wait closes a "System UI isn't responding" dialog with `CLOSE_SYSTEM_DIALOGS`, prints its last probe when the deadline passes, and runs again before a quick-boot snapshot save. A snapshot saved after a loaded settle could keep that dialog, and every restore then timed out the idle wait.
{% endif %}
{% if context.mobile %}- `docs/quality-gate.md` documents the Android emulator GPU settings (`BAUKIT_QA_ANDROID_GPU`, guest Vulkan off with `-feature -Vulkan`) and the `logcat-<device>.txt` and `chrome-version-<device>.txt` evidence a failed Maestro run keeps.
{% endif %}
{% if context.mobile and context.auth_enabled %}- Products with suite links pass `appScheme()` from `mobile/src/app-scheme.ts` to `createSuiteNativeIntentValidator` and build the native suite return origin as `${appScheme()}://suite`. With the product name there, the dev variant (`<app>-dev`) returned from the suite into the plain app. The template has no suite code of its own, so this is a product change; `baukit doctor` names each file that still uses the fixed scheme.
{% endif %}
{% if context.mobile and context.auth_enabled %}- `[auth] mobile_client_id` and `[auth] mobile_redirect_path` in `baukit.toml` name the mobile Keycloak client and its callback path (defaults `<app>-mobile` and `oauth`). `mobile/src/product.ts` exports them as `MOBILE_OIDC_CLIENT_ID` and `OIDC_REDIRECT_PATH`, and the lab variant's default client in `src/build-variant.ts`, the redirect in `src/sign-in/lab.ts` and `src/sign-in/local.ts` (or `src/auth.ts` for OIDC), `app/+native-intent.ts`, the realm files and the QA build scripts read them, so a product whose client or callback differs sets the two keys and re-renders instead of editing each file. Products that hand-edited these files set the keys, copy the listed files and drop their divergences.
{% endif %}
{% if context.mobile %}- `mobile/scripts/qa/android-env.sh prepare` checks `adb devices` first and exits 1 within seconds when an expected emulator is not booted, naming the serials and `sh scripts/quality-gate.sh --step android-boot`. Products that split `android-boot` from `android-e2e` waited about 20 minutes for `adb device not found` when they ran `--step android-e2e` alone. The template's own `android-e2e` still boots its emulators through `android-env.sh start` and is unchanged. Copy `android-env.sh` from the template.
{% endif %}
## [0.30.3] - 2026-11-24

{% if context.mobile %}- `mobile/scripts/qa/run-maestro.sh` passes the device target before `test`. Maestro 2.8.0 accepts `--host`, `--port` and `--device` only as global options, so since 0.30.2 every single-device Android run failed with `Unknown options: '--host', '--port'`. The sharded run moves `--device` in front of `test` as well. Products that hand-merged 0.30.2 must move `"${target[@]}"` in front of `test` at the three `run_maestro` calls (`run_declared_shard`, `run_device_flows` and the single-device loop), and `--device "$device_list"` in front of `test` in the sharded run.
{% endif %}{% if context.mobile %}- The mobile QA scripts (`services.sh`, `android-env.sh`, `build-android.sh`, `build-ios.sh`, `ios-env.sh`, `run-maestro.sh`, `docker-compose.qa.yml`) default PostgreSQL, Redis, the API, the operations listener and Keycloak to the product's own host ports from `baukit.toml` (`port_offset` or `[ports.*]`) plus 10000, rendered at generation. Before, every product defaulted to 15432, 16379, 18080, 19090 and 18081, so two products running `android-e2e` at once failed with `Bind for 127.0.0.1:15432 failed: port is already allocated`. Offset 0 keeps the old numbers. `BAUKIT_QA_*_PORT` and assignments in `mobile/scripts/qa/local.sh` still win. Copy the seven files from the template, or replace the five numbers in them by hand, and run `baukit check template-files --update`. For Clerk and WorkOS products doctor also accepts the derived QA Keycloak port.
{% endif %}{% if context.mobile and context.auth_enabled %}- `mobile/.maestro/smoke.yaml` runs `hideKeyboard` after typing the username as well as after the password, before it taps `kc-login`. On the first flow after boot Gboard opened cold while Chrome resized the page, so the tap on `password` landed in the username field. Add the same step to product flows that sign in through Keycloak.
{% endif %}{% if context.mobile %}- Products whose own tests run copies of `mobile/scripts/qa` scripts, as the template's `test_android.py` does, need three changes since 0.30.1. The test tree must contain `android-broker.sh`, because `android-env.sh` sources it unconditionally; the template test copies the whole `mobile/scripts/qa` directory. The test environment must set `BAUKIT_QA_ANDROID_MAX_LOAD_PER_CORE` high (the template test uses `1000`), or `start` and `boot` wait up to 600 s for the host load to drop. A fake `adb` rule that matches every `am start` also matches the Chrome warm-up `am start -W -a android.intent.action.VIEW -d about:blank -p com.android.chrome`{% if not context.auth_enabled %} that auth products run{% endif %}; match the app launch with `am start -n <package>/.MainActivity` instead.
{% endif %}{% if context.mobile and context.quality_strict %}- `scripts/quality-gate.sh` step `mobile-unit` runs `pnpm --dir mobile run test:scripts` before jest, in full and affected mode. The step ran only jest, so the `node --test` script tests in `mobile/scripts/*.test.mjs` never ran in the gate. `mobile/package.json` gains `test:scripts` with the `node --test` command that `test` ran, and `test` becomes `jest && pnpm run test:scripts`. Products that copied the template move their `node --test` list into `test:scripts` and copy `step_mobile_unit`.
{% endif %}{% if context.mobile %}- With `BAUKIT_QA_DEVICE_PORT_FORWARD` set to a kube context, `mobile/scripts/qa/android-env.sh` leases emulators through `pruefstand lease --port-forward <context>` running in the background, so a host outside the cluster network gets `127.0.0.1:<port>` serials. The CLI heartbeats and releases its own leases on SIGTERM, which `stop`, a failed `start` or the death of the calling shell sends. Before, passing `--port-forward` in `BAUKIT_QA_DEVICE_BROKER_ARGS` hung `start`. Copy `mobile/scripts/qa/android-broker.sh`.
{% endif %}
## [0.30.2] - 2026-11-24

{% if context.mobile and context.auth_enabled %}- `mobile/scripts/qa/android-env.sh` starts Chrome once on every emulator before the first flow (`am start -W` on `about:blank`, then Home) and once while it saves the quick-boot snapshot, so the snapshot holds a Chrome profile that has done its first start. Chrome's first start in a fresh profile kept its main thread busy for seconds; inside the login flow that work landed on the Custom Tab's first input, and at host load 60 to 80 Android raised `Input dispatching timed out` for `CustomTabActivity` after 5 s. `snapshot_schema` goes to 2, so the next start saves a new snapshot once.
{% endif %}{% if context.mobile %}- `mobile/scripts/qa/android-env.sh start` and `boot` wait while the host's 1-minute load per core is above `BAUKIT_QA_ANDROID_MAX_LOAD_PER_CORE` (2.0) before they launch local emulators, for up to `BAUKIT_QA_ANDROID_LOAD_WAIT_SECONDS` (600), then exit 1 with the load they saw. Leased emulators skip the wait.
- With more than one emulator, the first start saves the base AVD's snapshot, stops that emulator and copies its disks and snapshot into every extra AVD that has none, then quick-boots them all. Before, each extra AVD cold-booted and settled one after another, and a failed first start left it without a snapshot for the next start. The copy is skipped while another emulator runs the base AVD; that extra AVD then cold-boots as before.
{% endif %}{% if context.mobile %}- `mobile/scripts/qa/run-maestro.sh` runs Maestro on one emulator with `--host localhost --port <adb port>` in place of `--device emulator-<port>`. That path opens the emulator's adbd directly and skips the scan of every device on the shared adb server, so another session's offline emulator no longer aborts the run. The multi-device sharded run (`--shard-split`) still needs `--device` and keeps the retry and the named failure from 0.30.1. Products that run Maestro outside `run-maestro.sh` can pass the same two options.
{% endif %}{% if context.mobile %}- `mobile/scripts/qa/run-maestro.sh` passes the device count as the second argument of `qa_maestro_flows`, so a hook that shards flows no longer reads the script's `devices` array.
{% endif %}{% if context.mobile %}- `mobile/app.config.ts` reads `APP_VARIANT=dev|lab` together with `BAUKIT_LAB_DOMAIN`, the lab's DNS name (for example `lab.example.com`); a variant without the domain fails the build. `dev` appends `.dev` to the Android package and iOS bundle identifier, " Dev" to the name and `-dev` to the scheme, layers a dot over the launcher icon through `plugins/with-variant-badge.cjs` and points the API at `https://<app>-testing.<domain>`. `lab` keeps the plain identity and needs an https `EXPO_PUBLIC_API_URL`. Both take the version code from `BAUKIT_VERSION_CODE` or `GITHUB_RUN_NUMBER` (default 1){% if context.auth_enabled %} and sign in with `offline_access` against `https://id.<domain>/realms/<app>`, overridable with `EXPO_PUBLIC_LAB_OIDC_ISSUER` and `EXPO_PUBLIC_LAB_OIDC_CLIENT_ID`. Add `<app>-dev://oauth` next to `<app>://oauth` to the realm's mobile client. Sign-in redirects and `app/+native-intent.ts` read the scheme from `src/app-scheme.ts`; copy it with its test{% endif %}. Without `APP_VARIANT` the config is unchanged. To adopt it, copy `src/build-variant.ts`, `src/build-variant.test.ts`, `src/app-config-variant.test.ts`, `plugins/with-variant-badge.cjs` and `scripts/variant-badge.test.mjs`, add the test to the `test` script, set `allowImportingTsExtensions` in `tsconfig.json` if it is missing, and wrap your `app.config.ts` export as the template does{% if context.auth_enabled and not context.auth_oidc %}. Copy `src/sign-in/lab.ts`, `src/sign-in/lab.test.ts` and the `metro.config.js` resolver, which swaps the local adapter for `lab.ts` whenever `APP_VARIANT` is set; `scripts/check-production-bundle.mjs` then accepts the lab realm URL but still rejects loopback URLs and the local adapter{% endif %}. Run `baukit check template-files --update` after copying.{% if context.auth_enabled %} The dev variant signs in through `<app>-dev://oauth`, so add `"{{ context.app_name }}-dev"` to `allowedCustomSchemes` in `keycloak/realm-policy.json` and `"{{ context.app_name }}-dev://oauth"` to the mobile client's `redirectUris` in `keycloak/realm.json` and its `activeRedirectUris` in `keycloak/reconcile.json`; without them `keycloak_policy.py` fails with "uses custom scheme '{{ context.app_name }}-dev', which the policy does not allow".{% endif %}
{% endif %}
## [0.30.1] - 2026-11-24

{% if context.mobile %}- Each local Android emulator boots its own writable AVD: index 0 keeps `<avd>`, index N uses `<avd>-N`, created from `<avd>`'s `config.ini` and recreated when that file changes. Extra emulators no longer launch `-read-only` from one shared AVD, which crashed about once in 30 starts. Each AVD saves and keeps one quick-boot snapshot, so an extra AVD costs its own userdata and snapshot on disk, and the first start of an extra emulator cold-boots it. Copy `mobile/scripts/qa/android-env.sh` from the template and run `baukit check template-files --update`.
{% endif %}{% if context.mobile %}- `mobile/scripts/qa/run-maestro.sh` restarts a Maestro run once when another session's emulator on the shared adb server went offline while Maestro created its session. Maestro 2.8.0 opens every device adb lists (`Dadb.list` in `AndroidDeviceConnection.byId`), so the offline foreign device failed the run with `host:transport:emulator-5800: device offline` before any flow. When a flow had already started, or the retry hits it again, the run fails with a message naming the foreign serial.
{% endif %}{% if context.quality_strict %}- `scripts/quality-gate.sh` adds `require_node_modules DIR INSTALL_STEP`. A product step that runs Node code from `web/` or `mobile/` calls it first, so `--step scripts-tests` on a fresh checkout fails by naming `web-install` instead of with only `Cannot find module 'typescript'`.
{% endif %}{% if context.mobile %}- With `BAUKIT_QA_DEVICE_BROKER` set, `mobile/scripts/qa/android-env.sh` leases emulators through the `pruefstand` CLI instead of booting AVDs, passes `BAUKIT_QA_DEVICE_BROKER_ARGS` to `pruefstand lease`, heartbeats the leases and releases them on `stop` or when `start` fails. It exits 3 when the broker has no capacity. `scripts/gate.py` then skips the `BAUKIT_GATE_EMULATOR_CORES` reserve. Copy `mobile/scripts/qa/android-broker.sh`, `android-env.sh` and `scripts/gate.py`.
{% endif %}{% if context.quality_strict and context.backend %}- The `deploy-values` gate step runs its check under `uv`: `uv run --locked --project .` when the product has `pyproject.toml` and `uv.lock`, otherwise `uv run --no-project --with PyYAML==6.0.3`, so it no longer needs PyYAML on the system Python. `mise.toml` and the strict CI job install `uv` 0.12.3.
{% endif %}{% if context.backend %}- `postgres/initdb/10-owner.sql` makes `app` the owner of `current_database()`, not of a fixed name, so a compose stack with another `POSTGRES_DB` can mount the same script. Copy it from the template; existing volumes keep their owner.

{% endif %}
{% if context.mobile %}- `mobile/scripts/qa/android-env.sh` relaunches an emulator once when its process exits before it boots, which Android emulator 37.3.10 sometimes does with a segfault. The first log stays as `mobile/.qa/android-emulator*.log.crashed`. A boot timeout or a second early exit still fails with "did not boot". Copy the file from the template and run `baukit check template-files --update`.

{% endif %}
{% if context.mobile %}- The Android quick-boot snapshot is keyed on the system image, the AVD hardware settings, the GPU mode, the emulator features and `snapshot_schema` in `mobile/scripts/qa/android-env.sh`, not on the baukit version, so later re-pins reuse it. The first boot after this re-pin still saves one new snapshot, and saving it removes the older baukit snapshots of that AVD. Copy `mobile/scripts/qa/android-env.sh` and `android-adb.py` from the template and run `baukit check template-files --update`.

{% endif %}
## [0.30.0] - 2026-11-23

{% if context.backend %}- Breaking: migrations run as a non-superuser owner in the dev stack and the tests, as the migrate Job does on CNPG. Copy `postgres/initdb/10-owner.sql` and the `./postgres/initdb:/docker-entrypoint-initdb.d:ro` mount on the `postgres` service in `compose.yaml` from the template, switch local `__DATABASE__URL` values{% if context.mcp %} in `compose.yaml`{% endif %}{% if context.platform %}, `.env.example`{% endif %}{% if context.mobile %}, `mobile/scripts/qa/services.sh`{% endif %} and your `.env` to `postgres://app:app@...`, and run `docker compose down -v` once, because the init script runs only on an empty volume. Postgres tests call `baukit_test::start_postgres_with_owner_migrations(&migrations, baukit_test::PostgresMigrationOwner::default())` and connect with `owner_connection_url()`. Fix any migration that then fails: it needed a superuser and would fail in the cluster too. A migration that creates roles needs `[postgres] owner_create_role = true` and dedicated Postgres; `deploy/gitops/README.md` under "Migrations and roles" lists the rules. Run `baukit check template-files --update` after copying the files.

{% endif %}## [0.29.0] - 2026-11-23

{% if context.worker %}- A worker can opt in to the backing Redis the baukit-app chart gives the API: set `worker.redis.enabled: true` in the HelmRelease values. The worker then gets the URL in dedicated mode, the key namespace and `baukit.dev/redis-client` label in either mode, and the NetworkPolicy path to Redis. The default is `false`, so the chart renders your worker as before until you opt in. When you opt in, remove any `<APP>__RATE_LIMIT__REDIS_URL` entry from `worker.env` and any NetworkPolicy you added for worker Redis traffic in the same change; a second env entry with the same name fails server-side apply.

{% endif %}{% if context.quality_strict and context.backend %}- Breaking: the strict gate has a `deploy-values` step. It builds every `deploy/gitops` overlay with kustomize, lints and renders the baukit-app chart from the same Baukit checkout with each HelmRelease's values, and fails when a container sets one env name twice. It installs the helm and kustomize versions Baukit pins into `~/.cache/baukit/tools`. New products map `"deploy/**"` to `["deploy-values"]`, and `baukit doctor` fails when a `quality.affected` glob matches `deploy/gitops` without selecting `deploy-values` or the `fast` lane. Copy `scripts/quality-gate.sh` and `docs/quality-gate.md` from the template, run `baukit check template-files --update`, and set `"deploy/**" = ["deploy-values"]` in `[quality.affected]`; without a `deploy/**` glob every deploy change runs the full gate. Rename a product step already called `deploy-values` in `scripts/quality-gate.local.sh`, because the gate rejects duplicate step names.

{% endif %}{% if context.backend %}- `backend/Cargo.toml` requires sqlx 0.9.1. With 0.9.0 a `begin()` future cancelled mid-await left its pooled connection inside a server-side transaction, and the next borrower's statements ran in it. Set `version = "0.9.1"` on `sqlx` in `[workspace.dependencies]` and run `cargo update -p sqlx`.

{% endif %}## [0.28.8] - 2026-11-22

## [0.28.7] - 2026-11-22

## [0.28.6] - 2026-11-22

## [0.28.5] - 2026-11-22

{% if context.auth_enabled %}- `scripts/keycloak_policy.py` accepts `--environment-class lab` for a realm on a shared cluster. A lab policy must set `minimumTls` to `external` or `all` and `allowDevelopmentLoopbackHttp` to false. The check rejects default, known and literal user passwords (reference them as `$(env:NAME)` or `$(file:PATH)` for keycloak-config-cli), loopback redirect URIs, and an MCP audience mapper that does not name the policy's `mcpResourceUrl`. Copy the script and `scripts/tests/test_keycloak_policy.py` from the template; the `development` class is unchanged.

{% endif %}## [0.28.4] - 2026-11-19
- Every Linux job in `.github/workflows/ci.yml`{% if context.mobile %} and `native.yml`{% endif %} takes `runs-on` from the repository variable `BAUKIT_RUNS_ON`, a JSON label array such as `["self-hosted","lab","x64"]`, and falls back to `ubuntu-latest` when it is unset.{% if context.quality_strict %} The strict gate job reads `BAUKIT_GATE_RUNS_ON` instead, because it boots emulators and moves to self-hosted runners separately.{% endif %}{% if context.mobile %} The iOS job stays on `macos-15`.{% endif %} Run `baukit check template-files --update` after copying the workflows from the template.

{% if context.backend %}- New `deploy/gitops/{base,testing,production}` overlays and `release.yml`/`promote.yml` workflows. A push to `main` builds the backend images, pushes `<registry>/<app>-<process>:<sha12>`, signs them by digest with cosign and commits the pins to `deploy/gitops/testing/release.yaml` with the `GITOPS_DEPLOY_KEY` deploy key; the manual promote workflow verifies the signatures and copies the pins to production. Add `[gitops]` with `domain` and `registry` to `baukit.toml`, run `baukit generate gitops` to add the missing files, then `baukit check template-files --update`, and follow `deploy/gitops/README.md` for the GitHub environments, secrets and SOPS encryption. `baukit generate gitops` never overwrites a file; it lists the ones that differ from the template as kept.

{% endif %}
## [0.28.4] - 2026-11-19

{% if context.mobile %}- `run-maestro.sh` waits until `adb devices` lists every serial in state `device` right before each Maestro start, through the new `--await-devices` mode of `mobile/scripts/qa/android-adb.py`. Maestro 2.8.0 opens a transport to every serial the shared adb server on port 5037 lists and, when one of them is booting or offline, reports every Android device as not connected. A gate printed `Device emulator-5662 was requested, but it is not connected.` while that emulator kept streaming logcat. The run's own emulators get `BAUKIT_QA_DETACH_GRACE_SECONDS` to return, other sessions' emulators `BAUKIT_QA_BOOT_TIMEOUT_SECONDS`, and the failure names the serial and its adb state. After a failed Maestro run the script also prints any serial that is not ready. Copy `run-maestro.sh`, `android-adb.py`, `test_android.py` and, in strict products, `docs/quality-gate.md` from the template.

{% endif %}{% if context.quality_strict %}- `scripts/gate.py` reads `[gate.worker-caps]` from `baukit.toml` and exports the smaller of the lane's share and the cap for each listed variable, at least 1. A product whose Playwright allows at most 4 workers sets `E2E_WORKERS = 4` once instead of clamping in every step; hebkit's gate failed with `E2E_WORKERS must be an integer between 1 and 4, got '9'` where a step missed the clamp. The gate exits with status 2, and `baukit doctor` fails, on an unknown variable or a cap below 1. Copy `scripts/gate.py`, `scripts/gate.test.py` and `docs/quality-gate.md` from the template.

{% endif %}{% if context.mobile or context.web %}- `osv-scanner.toml` drops the ignore for GHSA-fc5q-39xc-vjpf (picomatch ReDoS). osv.dev withdrew the advisory on 2026-11-18. Delete the `[[IgnoredVulns]]` entry from your copy.

{% endif %}{% if context.quality_strict %}- `baukit check fuzz-lockfile` fails when git tracks `backend/fuzz/Cargo.lock` in a strict product. The fuzz step overwrites that file with `backend/Cargo.lock`, and six products committed a copy that drifted from the backend pins. Run `git rm --cached backend/fuzz/Cargo.lock`, then copy `scripts/quality-gate.sh`, `.github/workflows/ci.yml`, `CLAUDE.md` and `docs/quality-gate.md` from the template.

{% endif %}## [0.28.3] - 2026-11-19

{% if context.mobile %}- `run-maestro.sh` gives a run a longer stall window when one of its flows declares a longer wait. The new `mobile/scripts/qa/maestro-waits.py` reads the longest `timeout:` of any command in the flows, following `runFlow` files, and the window becomes that wait plus 60 s when that exceeds `BAUKIT_QA_MAESTRO_STALL_SECONDS`. The script prints the command that set it. Maestro prints nothing while a command waits, so an `extendedWaitUntil` with `timeout: 600000` that seeded 100,000 rows was stopped after 300 s as `the sharded run made no progress`. A `timeout:` that is not whole milliseconds, or a `runFlow` file that is missing or named by a variable, now fails the run with exit 2 before Maestro starts. Copy both files from the template.

{% endif %}{% if context.mobile %}- `mobile/scripts/qa/android-adb.py` waits `BAUKIT_QA_POLL_SECONDS` between probes in `--wait-for-boot` and `--wait-for-stop` too. Both slept a fixed 2 s, so a run that set a shorter interval still waited 2 s per probe there. `mobile/scripts/qa/test_android.py` pins both waits. Copy both files from the template.

{% endif %}{% if context.auth_enabled %}- `generated_keycloak_tools_pass_their_offline_checks` in `backend/tests/auth_conformance.rs` runs the four generated Keycloak tool suites by path: `test_erasure_roles.py`, `test_keycloak_policy.py`, `test_pkce_login.py` and `test_reconcile_keycloak.py` under `scripts/tests`. It ran `unittest discover -s scripts/tests`, which also picked up every product test kept in that directory, and under load one of those hit its own timeout inside the Rust test. A product that adds a Keycloak tool test lists it in `KEYCLOAK_TOOL_TESTS`. Copy the file from the template.

{% endif %}{% if context.quality_strict %}- `scripts/gate.py` starts `android-e2e` only after the `backend`, `fuzz` and `images` lanes finished, pass or fail. Under a full gate the emulator shared the host with Rust compiles, fuzzing and image builds, the guest load reached 72 on 24 cores, and Android typed key repeats such as `tteest` into Chrome Custom Tabs. The emulator core reserve starts once the wait is over. `[quality.waits_for]` in `baukit.toml` maps a step to the lanes or steps it starts after and replaces the default per step: `android-e2e = []` turns it off, `playwright = ["backend", "fuzz", "images"]` adds Playwright. The gate exits with status 2 on unknown names, `fast`-lane steps and waits that would never end. Copy `scripts/gate.py`, `scripts/gate.test.py` and `docs/quality-gate.md` from the template.

{% endif %}## [0.28.2] - 2026-11-18

{% if context.quality_strict %}- The `fuzz` step copies `backend/Cargo.lock` to `backend/fuzz/Cargo.lock` before it builds. The fuzz lockfile is not committed, so a fresh checkout resolved the newest Baukit patch while the backend pinned the older one, and `baukit doctor` failed on the mismatched pins once a new patch was published. Copy `scripts/quality-gate.sh` and `docs/quality-gate.md` from the template.

{% endif %}## [0.28.1] - 2026-11-18

- `run-maestro.sh` stops a sharded run only after adb has not listed an emulator as a device for `BAUKIT_QA_DETACH_GRACE_SECONDS` (30 s by default, 0 restores the immediate stop). A single `adb get-state` that timed out under load used to end the run as `died during the sharded run` while the emulator kept running.
- The generated `<product>-backend` client lists the realm default scopes plus `kc-admin-api` in `defaultClientScopes`. Keycloak 26.8.2 rejects Admin REST API tokens whose audience does not name the admin API, so the account deletion and user lookups failed with 401. `scripts/keycloak_policy.py` now fails when a client whose service account holds a realm-management role lacks `kc-admin-api`, or when the realm declares `clientScopes` without it.

## [0.28.0] - 2026-11-17

## [0.27.4] - 2026-11-17

## [0.27.3] - 2026-11-17

## [0.27.2] - 2026-11-17

## [0.27.1] - 2026-11-17

## [0.27.0] - 2026-11-17

- `backend/Dockerfile` builds on `rust:1.100.0-trixie` with the `docker/dockerfile:1.29.0` frontend. The distroless `cc-debian13:nonroot` and `debian:trixie-slim` runtime bases move to upstream's current digests. `mise.toml` and the CI workflow install Rust 1.100.0 to match.
- `web/Dockerfile` builds on `nginx/nginx-unprivileged:1.31.8-alpine`, pinned by digest, and sets `USER 101` explicitly. That is the uid the image already ran as, and it clears Trivy's DS-0002 finding.
- `compose.yaml` pins `postgres:18.7-alpine`, `redis:8.10.3-alpine` and the default `quay.io/keycloak/keycloak:26.8.2` by digest. The Keycloak theme check runs against 26.7.5 and 26.8.2. The mobile QA compose file uses the same redis pin.

## [0.26.7] - 2026-11-17

## [0.26.6] - 2026-11-17

## [0.26.5] - 2026-11-17

## [0.26.4] - 2026-11-16

{% if context.quality_strict %}- `scripts/gate.py` refuses to start when the filesystem of the checkout or of Docker's data root (`docker info`) has less free space than the floor. It exits with status 2, names the free GB and the floor, and lists the largest `target/` directories and Docker's reclaimable space. It deletes nothing. A run at 91% disk failed lanes 50 minutes in with pod evictions and Testcontainers timeouts. `[quality.disk] floor_gb` sets the floor (default 40), `[quality.disk.step_floor_gb]` raises it for named steps, and `BAUKIT_GATE_DISK_FLOOR_GB` overrides it for one run (0 turns the check off). `sh scripts/quality-gate.sh --step NAME` does not check. Copy `scripts/gate.py`, `scripts/gate.test.py`, `scripts/gate_locks.test.py`, `scripts/gate_disk.py`, `scripts/gate_disk.test.py` and `scripts/quality-gate.sh` from the template.
- `scripts/gate.py` passes ruff with the product's `ruff.toml` again: 0.26.2 left `CoreBudget` with six parameters and a line over 100 characters, so repo-lint failed after copying it. `CoreBudget` now takes an `EmulatorReserve(cores, steps)`.
{% endif %}
## [0.26.3] - 2026-11-16

## [0.26.2] - 2026-11-16

{% if context.quality_strict %}- `scripts/gate.py` holds the emulator cores back as soon as `android-e2e`'s dependencies pass, before the steps that waited on the same dependencies take the free cores. Before, those steps woke first and the emulator step often started on 2 or 3 cores, where a boot ANR or a 30-minute Gradle build followed. `[quality] emulator_steps` adds step names that get the same reserve, and `[quality] emulator_cores` sets its size (`BAUKIT_GATE_EMULATOR_CORES` still overrides it). Copy `scripts/gate.py` and `scripts/gate.test.py` from the template.
{% endif %}## [0.26.1] - 2026-11-16

{% if context.mobile %}- `mobile/scripts/qa/test_android.py` starts its fake worker with the absolute path of the running interpreter. It used `exec -a "$0" python3`, and when mise shims are on `PATH` the `python3` shim dispatches on `argv[0]`, saw `worker`, exited with "worker is not a valid shim" and printed nothing, so `test_stop_pid_file_stops_the_expected_binary_for_workers` failed. Copy the file from the template.
{% endif %}
## [0.26.0] - 2026-11-16

{% if context.quality_strict %}- `scripts/gate_locks.test.py` no longer expects `CorePool.acquire` to receive every released core at once. `release` closes its slot files one at a time, so a waiting step may start with any count between its minimum and its request, and `test_a_step_waits_for_its_minimum` failed under load with fewer cores than it asked for. The new `test_a_waiter_may_start_with_part_of_a_release` pins that case. Threads in the suite now finish through a `time.monotonic` loop instead of `Thread.join(timeout)`, which times out on the wall clock on Pythons without `sem_clockwait`. Copy the file from the template.
{% endif %}
## [0.25.2] - 2026-11-13

{% if context.mobile %}- `mobile/pnpm-workspace.yaml` no longer lists `minimumReleaseAgeExclude`. The Expo SDK 57 pins from 0.25.0 and 0.25.1 are past pnpm's one-day window, so a product deletes that block from its `mobile/pnpm-workspace.yaml`.
- `mobile/jest.config.cjs` runs `src/test/native-runtime.ts` first, which removes `BroadcastChannel` from the native Jest runtime as React Native has none. `@clerk/clerk-js` opens a `BroadcastChannel` at import and never closes it, and Node's keeps the process alive once it has a listener, so an in-band Jest run (`--maxWorkers=1`, which the gate's core pool hands out under load) finished every suite and then hung until the step was killed. Copy `src/test/native-runtime.ts` and `src/test/native-runtime.test.ts` and add the `setupFiles` entry.
{% endif %}{% if context.quality_strict %}- `scripts/gate_locks.test.py` waits with `time.sleep` instead of `threading.Timer` in `test_a_resource_step_waits_for_a_held_lock_before_taking_cores`. Python builds without `sem_clockwait`, such as mise's python-build-standalone 3.14, time out `Event.wait` and `Timer` on the wall clock, and WSL steps the wall clock forward by about 1.7 s every 34 s, so the 1.5 s timer could fire after 0.2 s and fail the test. Copy the file from the template.
- `scripts/gate.py` exits with status 2 before it runs anything when a `fast` step runs after a step that takes a lock, directly or through other steps. That step waits for the `fast` lane to pass, and the `fast` lane waited for it, so the gate hung with no step running. Move such a step out of the `fast` lane in `scripts/quality-gate.local.sh`.
{% endif %}
## [0.25.1] - 2026-11-13

{% if context.mobile %}- `mobile/package.json` builds expo-sqlite from source on Android (`expo.autolinking.android.buildFromSource`). The prebuilt expo-sqlite 57.0.5 AAR on npm ships `libexpo-sqlite.so` for arm64-v8a only, so on x86_64 emulators and 32-bit devices the app fails with `UnsatisfiedLinkError` when it opens the database and never leaves the loading screen. Copy the `autolinking` block into the product's `mobile/package.json` and rebuild the APK. Products that set an `expo.sqlite.*` Gradle property (SQLCipher, libSQL) already build from source.
{% endif %}- `baukit check template-files --update` adds rows for files a newer template owns (for example `scripts/gate_locks.py`) when the product already holds them unchanged, and lists template-owned files that differ as `template-owned but diverged, not tracked`. `baukit doctor` validates the step names in `[quality.resources]`, `[quality.shared_resources]`, `[quality.affected]` and `quality.light` against `scripts/quality-gate.sh --list-steps` for the standard profile too, so a product's own steps are accepted.

## [0.25.0] - 2026-11-13

{% if context.quality_strict %}- `scripts/gate.py` shares a host with other gates. With `BAUKIT_GATE_LOCK_DIR` set, a step that needs a host resource takes `<resource>.lock` with `flock` for its own run only. Steps queue FIFO tickets under the lock directory and take all their locks at once, exclusive or shared. A step that needs two locks is never starved by later single-lock steps, an exclusive step is never starved by a stream of shared ones, and no step holds one lock while it waits for another. Steps that hold a resource start after the `fast` lane passed (otherwise `not run (fast lane failed)`). `android-boot`, `android-e2e` and `android-debug` take `android`, and `android-e2e` also takes `docker-network` exclusively because the QA backend starts with `docker compose up`, which creates a network and makes host Chromium report `ERR_NETWORK_CHANGED`. Steps behind `scripts/with-loopback-network.sh` take no lock. `[quality.resources]` and `[quality.shared_resources]` in `baukit.toml` add steps or lanes per resource name, and `baukit doctor` checks the names. With `BAUKIT_GATE_CORE_POOL` set, `CoreBudget` takes cores from `slot-<n>.lock` files shared by every gate on the host, and each step's share is the pool divided by the live gates. Step lines report lock waits and free pool cores. New files `scripts/gate_locks.py` and `scripts/gate_locks.test.py`, run by the `scripts-tests` step. Copy them with `scripts/gate.py` and `scripts/quality-gate.sh`.
- `scripts/gate.py` keeps `step-seconds.txt` in the log directory: each passed step's last run time, merged across full, `--affected` and `--step` runs. A heavy step with history gets a core share in proportion to its expected time among the heavy steps still to start, between the even share and the cores free at its start. Steps without history keep the even share. Copy `scripts/gate.py`.
{% endif %}- `mise.toml` pins Rust 1.99.0 with clippy and rustfmt, and the four `toolchain: stable` inputs in `.github/workflows/ci.yml` install 1.99.0. New `scripts/rust-toolchain-pin.test.py` fails when a workflow installs another toolchain than the mise pin, the MSRV output or nightly; `ci.yml` and the strict `scripts-tests` step run it.
{% if context.mobile %}- Generated mobile apps pin Expo 57.0.33, expo-router 57.0.31, expo-linking 57.0.14, expo-sqlite 57.0.5 and expo-constants 57.0.24 in the catalog{% if context.auth_enabled %}, and expo-auth-session 57.0.17{% endif %}, the SDK 57 patch releases of 2026-11-12, and list them in `minimumReleaseAgeExclude` until they are a day old.
{% endif %}
## [0.24.0] - 2026-11-12

{% if context.mobile %}- `expo install --check` in `ci.yml` and the `expo-doctor` step of `scripts/quality-gate.sh` run with `EXPO_OFFLINE=1`, so they compare against `bundledNativeModules.json` of the installed `expo` package instead of the Expo API. The API's expected versions change when Expo ships a patch, and `minimumReleaseAge` blocks a product from following it for a day. Expo prints "Dependency validation is unreliable in offline-mode", yet a native module outside the installed SDK's range still fails both checks. The step runs `expo-doctor@1.20.5` instead of the newest release. Copy both changes.
- Mobile QA scripts. `android-env.sh` starts every emulator with `-feature -Vulkan` (joined with `-WiFiPacketStream` by a comma when `BAUKIT_QA_ANDROID_WIFI` is on), so Chrome renders through GL instead of a guest SwiftShader Vulkan instance, which crashed qemu during Custom Tab flows. The quick-boot snapshot name includes the feature string, so existing snapshots rebuild once. `run-maestro.sh` records `logcat -v threadtime` and the Chrome `versionName` per device for every Android run, keeps them as `maestro-debug/logcat-<serial>.txt` and `chrome-version-<serial>.txt` for a device whose flow failed, deletes them when the run passes, and stops the logcat processes on exit. `services.sh` `stop_pid_file` takes a third argument, the expected binary: `stop_pid_file "$state_dir/worker.pid" worker "$qa_backend_bin_dir/worker"`. It stops the PID only when its command line starts with that path, and otherwise refuses and keeps the pid file. Update every `qa_stop_services` hook that calls it. The iOS Maestro step in `native.yml` puts `$HOME/.maestro/bin` on `PATH` and calls `maestro`.
{% endif %}{% if context.web %}- `scripts/with-loopback-network.sh` is new. The strict gate's `playwright` and `webkit-repeats` steps run through it, so Chromium no longer sees host network changes (`net::ERR_NETWORK_CHANGED`, "Unable to preload CSS") while another gate creates or removes Docker networks. It exits 69 when unprivileged user namespaces are unavailable instead of falling back to the host network; `BAUKIT_E2E_NETWORK_ISOLATION=0` is the explicit opt-out. `ci.yml` runs `sysctl kernel.apparmor_restrict_unprivileged_userns=0` before the gate and before the `e2e-web` Playwright step, which now uses the helper too. Copy the script and its `with-loopback-network.test.py`, and add the sysctl step to any job of your own that runs the Playwright gate.
{% endif %}
## [0.23.0] - 2026-11-12

{% if context.web %}- `web/scripts/check-production-bundle.mjs` builds through Vite's JS API and a `generateBundle` plugin, and names each finding as `chunk: module: marker`. The loopback URL marker applies only to modules outside `node_modules`, because third-party code such as `@tanstack/router-core` carries a `http://localhost` fallback. The Keycloak adapter, `/realms/` and env canary markers still apply to every module, and source maps are scanned for them only. Copy the new script and its test.
{% endif %}{% if context.auth_enabled and context.backend %}- `scripts/pkce-login.py` captures a callback with a non-http scheme such as `app://oauth` in `http_error_301`, `_302`, `_303`, `_307` and `_308`, because urllib rejects the scheme before `redirect_request` runs. `scripts/pkce_session.py` is new. Its `sign_in(SignIn(issuer, client_id, redirect_uri, credentials))` loads `pkce-login.py`, registers it in `sys.modules` and builds the `LoginRequest`, so seed and smoke scripts stop depending on the dataclass fields. `Credentials(username, password)` is per call, so one process can sign in as several users, and `Credentials.from_env()` reads `BAUKIT_QA_USERNAME` and `BAUKIT_QA_PASSWORD`, and `Credentials.development()` fills a missing one from the `test` user in `keycloak/realm.json`, so the script holds no password.
- `[auth] login_theme` in `baukit.toml` names the Keycloak login theme. It defaults to `baukit-accessible` and feeds `keycloak/realm.json` and `KEYCLOAK_LOGIN_THEME` for `scripts/test-keycloak-theme-patches.sh`, which `scripts/keycloak-theme.browser.mjs` asserts against the realm.
{% endif %}{% if context.quality_strict %}- `[quality.ci]` in `baukit.toml` sets `timeout_minutes` (default 90) and an `env` table for the `strict-quality` job in `.github/workflows/ci.yml`. A `PLAYWRIGHT_BROWSERS_PATH` entry there replaces the step-level default of the browser install step.
{% endif %}- `scripts/lockfiles.sh` sources `scripts/lockfiles.local.sh` when it exists. The file can redefine `lockfile_run` (for example `lockfile_run() { mise exec -- "$@"; }`) to wrap every lockfile command, and define `extra_lockfiles` for product lockfiles such as `mise lock` or `uv lock`. `scripts/lockfiles.test.py` covers both hooks.
{% if context.web or context.mobile %}- `ci.yml`, `native.yml`, the `Makefile`, `lockfiles.sh`, `CLAUDE.md` and the READMEs call `corepack pnpm` from the package directory instead of `corepack pnpm@12.9.1`, so `packageManager` in `package.json` decides the version. The lockfile bootstrap in `baukit new` does the same.
{% endif %}
{% if context.mobile %}- Mobile QA scripts. `android-env.sh` passes `--force-renderer-accessibility=complete` to Chrome (products with auth), closes inherited file descriptors above 2 before it starts an emulator, stops the emulators it started when `start` or `boot` fails, and ends the boot wait as soon as the emulator process is gone. It has a `reverse <serial>` action, which `run-maestro.sh` runs before every flow, and `qa_apk_variant` (`debug` embeds the JS bundle and installs `app-debug.apk`). `run-maestro.sh` has the hook `qa_maestro_flows index` (a flow list per device, one Maestro process per flow) and `qa_ios_bundle_id`, which `ios-env.sh` reads too, and it no longer falls back to `$HOME/.maestro/bin/maestro`. `services.sh` has `stop_pid_file PID_FILE NAME` for `qa_stop_services` hooks. `build-android.sh` and `build-ios.sh` call `corepack pnpm` without a version so `packageManager` decides, run from `mobile/`, and `BAUKIT_QA_GRADLE_LOG` tees the Gradle output. Every script defines `root`, `state_dir` and `api_port` before it sources `local.sh`; `local.sh.example` lists the hooks. `BAUKIT_QA_ANDROID_ACCEL` sets the emulator `-accel` mode. `build-ios.sh` defaults the scheme to `expo.name` from `mobile/app.json`, else the Xcode workspace name. Both build scripts point `TMPDIR` at `state_dir/metro-cache/<checksum of the EXPO_PUBLIC_* environment>`, so builds with different public values do not share a Metro cache. `services.sh` reads the backend binaries from `qa_backend_bin_dir` (default `backend/target/debug`). A product whose e2e step relied on `android-env.sh prepare` starting the services (0.22.0 stopped that) must run `services.sh start` itself.
{% endif %}

## [0.22.0] - 2026-11-11

{% if context.mobile %}- Mobile QA scripts take product needs through `mobile/scripts/qa/local.sh`. `android-env.sh` has `boot` and `prepare` next to `start`, a `qa_app_id` (or `BAUKIT_QA_APP_ID`) shared with `run-maestro.sh`, `qa_apk_path`, `qa_build_apk`, `qa_prepare_device`, the `api_port` and `keycloak_port` variables, and `BAUKIT_QA_ANDROID_GPU`. `services.sh` has `qa_services_start` and `qa_services_stop`, `BAUKIT_QA_STATE_DIR` and `BAUKIT_QA_COMPOSE_OVERRIDE`, and its Keycloak pins `KC_HOSTNAME` to the QA port. `run-maestro.sh` has `qa_maestro_shards`, `qa_maestro_env` and `BAUKIT_QA_MAESTRO_DEADLINE_SECONDS`. `build-android.sh` and `build-ios.sh` have `qa_build_env`, `qa_install_dependencies` and, for Android, `qa_gradle_args`; the debug assemble builds one ABI. `android-sdk.sh` reads `BAUKIT_QA_ANDROID_PLATFORM` and `BAUKIT_QA_ANDROID_BUILD_TOOLS`, and iOS builds read `BAUKIT_QA_IOS_SCHEME`. A failing `adb install` or `am start` prints the adb output.
{% endif %}
## [0.21.0] - 2026-11-10

- `scripts/process_runner.py` has `start_process(command, **options)`, which returns a `subprocess.Popen` for long-lived children such as port-forwards and supervised children that need `pass_fds` or `start_new_session`. It resolves the executable with `shutil.which` and holds the single reasoned `noqa: S603`. `run_process` keeps its signature and is built on `start_process` with the `subprocess.run` semantics: `capture_output` and `input` map to pipes, `timeout` kills the child and re-raises, `check` raises `CalledProcessError`. {% if context.quality_strict %}`quality-gate.sh` runs the new `scripts/process_runner.test.py`. {% endif %}Replace direct `subprocess.Popen` calls in your own scripts with `start_process`.

## [0.20.0] - 2026-11-10

{% if context.mobile %}- `android-adb.py --wait-for-stop` waits for adb to drop the emulator and for the qemu process to release its console port, within `BAUKIT_QA_STOP_TIMEOUT_SECONDS` (120 seconds by default). It used the 10 second device probe timeout as the stop deadline, and a loaded host took longer. It retries a timed-out probe and prints the measured stop time.
{% endif %}{% if context.auth_enabled and context.mcp and (context.web or context.pwa) %}- The containerized `backend` compose service now serves `/auth/session` instead of disabling it. Keycloak runs with `KC_HOSTNAME=http://localhost:<keycloak port>` and `KC_HOSTNAME_BACKCHANNEL_DYNAMIC=true`, so the token issuer stays browser-facing while discovery and JWKS URLs name `keycloak:8080`. `BrowserSessionSettings.internal_issuer` (`AUTH__BROWSER_SESSION__INTERNAL_ISSUER`) feeds `BrowserSessionConfig::with_internal_issuer`. Products with their own compose file add the same two Keycloak variables and the setting.
{% endif %}{% if context.auth_enabled and (context.web or context.pwa) %}- Local sign-in origins use `localhost` only and cover every declared web port. `keycloak/realm.json`, `keycloak/reconcile.json`, `config/local.toml` (`cors_allowed_origins`), the `BrowserSessionSettings` default `redirect_uris` and the Clerk `authorized_parties` default list `http://localhost:<port>` for `ports.web`, `ports.web_preview` and every `ports.web_extra` entry, not only the web port. {% if context.web %}`web/e2e/playwright.config.ts` serves the preview on `localhost` instead of `127.0.0.1`, because the session cookie is same-site and `127.0.0.1` is a different site (ADR 0006).{% endif %} Re-render those files and replace `127.0.0.1` web origins in your own config.
{% endif %}{% if context.auth_enabled and context.backend %}- `PostgresProfileErasure` calls `baukit_auth::browser_session::erase_subject_browser_sessions`, so `DELETE /me` removes the subject's rows from `baukit_auth_browser_sessions`. Add the same call to products that already customized `PostgresProfileErasure`, and add `baukit-auth` with the `browser-session` feature to the postgres crate.{% if context.worker %} The durable worker's daily purge job is now `RetentionJobHandler` in the `retention` module (job type `retention.purge`, formerly `PlatformJobHandler` and `platform.receipts.purge`) and also calls `purge_browser_sessions`. Drain the old job type before upgrading.{% endif %}
{% endif %}{% if context.quality_strict %}- The `scripts/quality-gate.sh` fuzz step passes a scratch directory under the gate log directory as the first corpus argument and `backend/fuzz/corpus/<target>` second. libFuzzer writes new inputs only to the first directory, so the tracked seeds stay unchanged.
{% endif %}{% if context.quality_strict %}- `scripts/gate.py` lists steps that never ran as "not run" in the failure summary instead of raising `KeyError`.
{% endif %}{% if context.quality_strict %}- `scripts/quality-gate.sh` exports `PYTHONDONTWRITEBYTECODE=1`. `gate.py` imports `process_runner`, and the `scripts/__pycache__` file that import left behind showed up as an untracked change, so `--affected` always fell back to the full gate. The backend `.gitignore` also ignores `__pycache__/`.
{% endif %}- `scripts/osv-resolved-lock.py` writes a `Cargo.lock` with only the packages `cargo tree --all-features --target all` can reach, and `scripts/repo-lint.sh osv` scans that file. Lockfile-only optional dependencies such as `libsqlite3-sys` no longer raise advisories.{% if context.backend %} Delete the RUSTSEC-2026-0459 entry from `osv-scanner.toml`.{% endif %}{% if context.quality_strict %} `quality-gate.sh` runs `scripts/osv-resolved-lock.test.py`.{% endif %}
{% if context.mobile %}- Product hooks for the QA scripts. `mobile/scripts/qa/local.sh` is sourced by `services.sh`, `android-env.sh` and `run-maestro.sh` and can set `qa_emulator_args`, `qa_reverse_ports` and `qa_maestro_args`{% if context.backend %}, `qa_compose_services`, `qa_cargo_bins` and the functions `qa_services_env`, `qa_before_api`, `qa_after_api` and `qa_stop_services`{% endif %}. Move product edits of these scripts into `local.sh` and re-render the template files.
{% endif %}{% if context.web %}- `web/scripts/generate-tokens.mjs` reads `web/src/design-tokens.json` when it exists (parsed with `parseTokens` and checked for WCAG contrast) and appends `web/src/tokens.extra.css` to `tokens.css`. It exports `generateTokens` and has a `node --test` suite.
{% endif %}{% if context.web or context.mobile %}- `scripts/quality-gate.sh` reads the pnpm version from `packageManager` in `{{ "web" if context.web else "mobile" }}/package.json` instead of hardcoding `pnpm@12.9.1`, and `scripts/preflight.sh` runs `corepack pnpm` from the package directory. A pnpm bump now needs one edit.
{% endif %}- `scripts/quality-gate.sh` `use_baukit_root` adds the baukit checkout to `MISE_TRUSTED_CONFIG_PATHS`, so rustc or kache children started inside the clone do not stop at an untrusted `mise.toml`.
{% if context.pwa and not context.web %}- `scripts/quality-gate.sh` sets `PLAYWRIGHT_BROWSERS_PATH` for `pwa` products without `web`, so worktrees share one browser cache.
{% endif %}- `ruff.toml` no longer ignores `E501`, `S603`, `S607`, `PLR2004` or `PLR0913` globally. Test files (`test_*.py`, `*.test.py`) ignore `E501`, `S603`, `S607` and `PLR2004`. Other scripts resolve executables with `shutil.which`, route every subprocess call through `scripts/process_runner.py` (one reasoned `noqa: S603`), and bundle long argument lists into dataclasses (`BudgetLimit`, `RunContext`, `LoginRequest`, `RecoveryHooks`). Copy the new `ruff.toml` and fix the findings.
{% if context.backend and context.mcp %}- `backend/tests/config_loader.rs` runs the `api`, `migrate` and `worker` subprocesses with a 60 s limit. A command that exceeds it is killed and the test fails with the child's process state instead of waiting forever.
{% endif %}
- `baukit-check.toml` allows `noqa = 1` (`noqa = 2` with a mobile app). The only `# noqa: S603` sites are `scripts/process_runner.py`, which every script imports to run subprocesses, and `mobile/scripts/qa/android-adb.py`. Copy the new `baukit-check.toml`.

## [0.19.2] - 2026-11-08

{% if context.auth_enabled and (context.web or context.pwa) %}- `[auth.browser_session]` takes `ip_rate_per_minute`, `ip_rate_burst`, `session_rate_per_minute` and `session_rate_burst` (env `..__AUTH__BROWSER_SESSION__IP_RATE_PER_MINUTE` and so on). Defaults are the ADR 0006 limits; `config/local.toml` raises them so Playwright suites do not hit HTTP 429 on `/auth/session/exchange`. Copy the four keys from the new `local.toml` and the `rate_limits` wiring from `browser_session.rs`.
{% endif %}{% if context.web or context.mobile %}- `@baukit/suite-client`: `SuiteTransport.request` returns `Promise<unknown>` and `SuiteClient` validates every response, rejecting with `SuiteResponseError`. Drop the type parameter and any `as T` cast from custom transports and test fakes.
{% endif %}{% if context.backend %}- Security: `osv-scanner.toml` ignores RUSTSEC-2026-0459 until 2027-02-07. sqlx 0.9.0 caps `libsqlite3-sys` below the fixed 0.39, and no backend crate compiles it. Copy the entry from the new template.
{% endif %}{% if context.mcp %}- Security: `rmcp` is 3.9.0, past RUSTSEC-2026-0457, RUSTSEC-2026-0461 and RUSTSEC-2026-0462. Run `cargo update -p rmcp`.
{% endif %}- `scripts/repo-lint.sh` passes `--force-exclude` to `ruff check` and `ruff format --check`, so `exclude`, `extend-exclude` and `[format] exclude` in `ruff.toml` apply to the explicit file list.
{% if context.auth_enabled and context.pwa and not context.web %}- Security: the backend serves the `/auth/session` browser-session routes, CORS for the web origin and the `-web` Keycloak client when `pwa` is set, not only with `web`. Migration: copy the `[auth.browser_session]` and `[http] cors_allowed_origins` blocks from `local.toml`, the `BROWSER_SESSION` compose variable, the `browserSession` Helm values and the `-web` client in `keycloak/realm.json`, then run the browser-session migration.

{% endif %}
## [0.19.1] - 2026-11-08

{% if context.quality_strict %}- `scripts/quality-gate.sh` fuzz step passes the nightly host triple to `cargo fuzz`. The prebuilt cargo-fuzz from `install-action` is a musl binary and otherwise builds the targets for musl, which ASan rejects.

{% endif %}## [0.19.0] - 2026-11-07

- Breaking: `AGENTS.md` is a symlink to `CLAUDE.md`, and `CLAUDE.md` ends with the managed Baukit agent-rules block. Replace an existing `AGENTS.md` copy with the symlink, then run `baukit check agent-docs --fix --root .` from the matching Baukit checkout.
- Every workflow action is pinned to a commit SHA with its version in a comment, and every checkout sets `persist-credentials: false`. `rust-toolchain` and `install-action` take the toolchain and tools as inputs.
- `scripts/repo-lint.sh` runs actionlint and zizmor, gitleaks, osv-scanner, ruff and shellcheck. `mise.toml` pins those tools, and `zizmor.yml`, `osv-scanner.toml`, `ruff.toml` and `.shellcheckrc` configure them. {% if context.quality_strict %}`scripts/quality-gate.sh` runs them as fast-lane steps{% else %}A `repository-lint` CI job runs them{% endif %}.
- {% if context.quality_strict %}The `baukit-check` gate step{% else %}The `baukit-doctor` CI job{% endif %} runs `baukit check agent-docs file-size suppressions lints patterns observability template-files` with the limits in `baukit-check.toml`.

{% if context.backend %}- Breaking, security: the `items` example is owner-scoped. Migration `0001_create_items.sql` adds `owner_id` and an `(owner_id, id)` index, every query carries the owner in its `WHERE`, ids are server-generated UUIDv7, and `tests/support/owner_scoping.rs` checks that a second owner can neither read, list, update nor delete the first owner's item. Products that kept the example routes either delete them or apply the same scoping.
- Security: every route sits inside the rate limiter. `rate_limits.rs` holds `rate_limited` and{% if context.auth_enabled %} `protect_routes`. Item writes get a per-identity quota, and the browser-session, MCP and platform routes get the IP limiter on their own{% else %} wraps the whole API in an in-memory IP limiter, so each replica keeps its own buckets{% endif %}.
- The backend carries the strict lint baseline: workspace `[lints]`, `clippy.toml` (raw `reqwest` is a disallowed method) and `deny.toml`. `scripts/repo-lint.sh deny` runs cargo-deny.
- `backend/fuzz` is a cargo-fuzz crate with an `item_body` target. Add a target for every parser or decoder the product owns.
- `deploy/observability` ships an API dashboard and recording rules that `baukit check observability` keeps in sync with the emitted metrics.
{% endif %}{% if context.auth_clerk and context.mobile %}- The Clerk verifier sets `allow_missing_azp: true`, because native requests carry no `Origin` and Clerk then omits `azp`. A present `azp` must still be allowed.
{% endif %}- `.baukit/template-files.toml` lists the template-owned files that `baukit check template-files` compares against the template. Product-specific gate steps go in `scripts/quality-gate.local.sh` (`local_steps` plus `step_<name>`), not in template-owned scripts.
- `CLAUDE.md` lists the installable Baukit agent skills.
{% if context.web or context.mobile %}- Web and mobile lint with `@baukit/eslint-config`; every `eslint-disable` needs `-- reason`. The Content Security Policy comes from `@baukit/csp`: delete the local `policy.ts` copy and keep only the origins.
{% endif %}{% if context.web %}- `vite preview` and the chart's `web` component send `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy` `same-origin` next to the CSP. COEP stays unset.
{% endif %}
## [0.18.3] - 2026-11-07

{% if context.web %}- Web source reads every `VITE_*` key with static `import.meta.env.VITE_...` access, declared in `web/src/vite-env.d.ts`, so Vite no longer inlines the whole env object into a production bundle.
{% endif %}{% if context.mobile %}- `mobile/eslint.config.js` turns on `preserve-caught-error` and `@typescript-eslint/no-dynamic-delete`, and `pnpm lint` checks `mobile/scripts`.
{% endif %}{% if context.mobile and context.quality_strict %}- `baukit.toml` maps `mobile/.maestro/**` to `android-build` and `android-e2e`, so `--affected` runs the flows for a Maestro-only change.
{% endif %}
## [0.18.2] - 2026-11-06

{% if context.mobile %}- Metro refuses to bundle when a `mobile/.env*` file sets `BAUKIT_QA_BUILD`, and the app config passes `apiBaseUrl` only when `EXPO_PUBLIC_API_URL` is set.
{% endif %}{% if context.mobile and context.auth_enabled and not context.auth_oidc %}- The app config passes the local realm only to development and QA builds. The mobile bundle check exports with a clear Metro cache and `mobile/.env` loaded, scans the Expo web export and copied public files, and runs in the standard profile's CI.
{% endif %}{% if context.mobile %}- `mobile/pnpm-workspace.yaml` no longer lists the Expo SDK 57 patch releases in `minimumReleaseAgeExclude`. They are past pnpm's one-day window.
{% endif %}{% if context.mobile %}- Every adb call in the Android QA scripts has a timeout. `BAUKIT_QA_ADB_TIMEOUT_SECONDS` sets it, 60 seconds by default and 10 for device probes.
{% endif %}{% if context.quality_strict %}- `scripts/quality-gate.sh` runs `baukit doctor` as the `baukit-doctor` step in the `fast` lane.
{% else %}- `.github/workflows/ci.yml` gains a `baukit-doctor` job that runs `baukit doctor` from the pinned Baukit tag.
{% endif %}- `scripts/setup.sh` warns about keys in a local `.env` that its `.env.example` no longer lists, and leaves them in place.


## [0.18.1] - 2026-11-05

{% if context.mobile %}- Generated mobile apps pin Expo 57.0.32, expo-router 57.0.30, expo-constants 57.0.23 and @expo/metro-runtime 57.0.18, the SDK 57 patch releases, and list those releases in `minimumReleaseAgeExclude` until they are a day old.

{% endif %}## [0.18.0] - 2026-11-05

- Breaking: `scripts/keycloak_policy.py` accepts only the `development` class, and the production realm and policy fixtures are gone (BK21). Keycloak runs only for local development and E2E.

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

- Clerk MCP accepts dynamically registered clients when `mcp.required_scopes` is set and `mcp.oauth_client_id` is not. Startup requires one of them.

- Clerk and WorkOS products run their production verifier locally against a provider-shaped Keycloak realm. `config/local.toml` sets `auth.issuer` and `auth.jwks_uri` to the realm, and the ignored `local_realm` backend test checks the token shape against both verifiers.

## [0.15.0] - 2026-11-02

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

- Breaking: web QA submit targets in `e2e/qa.config.ts` take `fields` and `invalidField`. The single-field `QaLegacySubmitField` shape and its helpers are gone. Generated local-data docs no longer describe claiming unowned legacy stores.

## [0.13.0] - 2026-10-31

## [0.12.2] - 2026-10-31

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

{% if context.mcp %}- Document PAT verification, owner mapping, and mixed PAT/OIDC policies for
  remote MCP. Verified PAT grants now permit matching MCP tools. Review existing
  grants when upgrading a product that already accepts PATs at `/mcp`.

{% endif %}## [0.11.0] - 2026-10-22

{% if context.backend or context.mobile %}{% if context.mobile %}- Generated mobile apps pin Expo 57.0.31 and expo-router 57.0.29 to match Expo dependency checks.
{% endif %}{% if context.backend %}- Generated backend image builds use `scripts/docker.sh` to key Cargo target caches by checkout. Plain Compose retains the app-name default. Override either with `CARGO_TARGET_CACHE_PREFIX`.
{% endif %}
{% endif %}## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

{% if context.mobile %}- Generated mobile apps pin Expo 57.0.30, expo-auth-session 57.0.16, expo-constants 57.0.22,
  expo-linking 57.0.13, expo-router 57.0.28 and @expo/metro-runtime 57.0.17.

{% endif %}## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

{% if context.backend %}- End the generated backend Dockerfile with a single newline.
{% endif %}- Keep generated files free of trailing whitespace and blank lines at EOF.

## [0.10.9] - 2026-10-10

{% if context.backend %}- Use Dockerfile frontend 1.28.0 for the backend image.
- Generate the backend Dockerfile from `baukit.toml`. Regenerate it with `baukit generate dockerfile` after a Baukit update.
- Pin Rust and distroless base images by digest. Set `SQLX_OFFLINE=true` during image builds.

{% endif %}## [0.10.8] - 2026-10-09

{% if context.web %}- Wait for overlay triggers to be enabled before focusing them in the generated keyboard test.

{% endif %}{% if context.backend %}- Build the backend image with BuildKit cache mounts for the cargo registry and `target/`. Image layers now hold only the binaries, not a full release `target/` per build.

{% endif %}## [0.10.7] - 2026-10-09

{% if context.mobile %}- Preserve Android QA health events and matching ANR summaries in stderr and `.qa/android-health.log`.

{% endif %}## [0.10.6] - 2026-10-09

{% if context.backend and context.auth_enabled %}- Choose `IdentityRetention::Delete` explicitly when building the erasure service. Generated products still delete the provider account.
- Fix Rust formatting in authenticated backends without MCP.

{% endif %}## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

{% if context.auth_oidc and context.mcp %}- Remove stale MCP audience mappers and callback URIs during Keycloak reconciliation. Preserve callback order returned by Keycloak.

{% endif %}## [0.10.3] - 2026-10-08

{% if context.backend %}- Wait for PostgreSQL to accept TCP connections before reporting Compose health.
{% endif %}{% if context.mobile %}- Pin Android CI to Temurin 25.0.4.1+1 and launch QA through MainActivity.
{% endif %}

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Place shipped authentication template notes under their release headings.

- Share configuration collection declarations across the API, migration and worker commands. Parse MCP hosts and origins as JSON arrays in every command.

{% if context.auth_oidc %}- Reconcile Keycloak client scopes and their bindings. Repair scope protocol mappers through their Admin API endpoints.

{% endif %}{% if context.mcp or context.mobile %}{% if context.mcp %}- Breaking: MCP tool adapters receive a request cancellation token and stop pending item reads when cancelled.
- Set the product name, version and instructions in MCP server discovery. Keep the default annotations explicit in tool declarations.
{% endif %}{% if context.mobile %}- Breaking: Native checks now require Java 25. Use Android command-line tools 23.0 in QA. Select the macOS download for the host CPU.
- Grant native access to Android's Prefab tool during native builds. Preserve existing JVM options.
{% endif %}
{% endif %}{% if context.mcp %}- Keep product-crate imports in their own group in the MCP crate and drift test, so rustfmt output no longer depends on the product name.

{% endif %}## [0.9.0] - 2026-10-07

{% if context.mcp %}- Fetch signing keys over the internal network while keeping the public issuer. Add an optional backend Compose profile.
- Register MCP services through `McpServices`.
- Check tools, resources and prompts in MCP definition drift tests. Accept default tool errors in the example output schema.
- Fix rustfmt drift in the MCP router wiring for short product names.

{% endif %}{% if context.web %}- Measure the screen and scroller together in the browser geometry check, so navigation animation cannot mix two layouts.

{% endif %}{% if context.mobile %}- Control the animation frame in the Android navigation test so dismissal checks do not depend on elapsed time.
- Keep Android QA ownership until cleanup succeeds. Verify emulator shutdown and report service teardown failures.
- Reject System UI and keyboard crashes or ANRs after boot and before Maestro flows.
- Limit the Android software renderer to four threads by default. Set `BAUKIT_QA_RENDERER_THREADS` to change the limit.

{% endif %}{% if context.auth_enabled %}- Generate {{ context.auth_provider }} sign-in, token verification and profile deletion. MCP applies the provider's OAuth token binding.

{% endif %}## [0.8.0] - 2026-10-07

{% if context.mcp %}- Breaking: `--mcp` now generates the Rust remote server and requires `--backend --auth oidc`. The TypeScript stdio template and its authentication flags are removed. Set `capabilities.mcp = true` after migrating existing tools.
{% endif %}{% if context.mobile %}- Update Expo SDK 57 patches to match Expo compatibility checks.
{% endif %}

{% if context.mcp %}- Add an opt-in Rust remote MCP template with Keycloak PKCE, deployment routes, exact Host and Origin configuration, and scoped service ports.

{% endif %}## [0.7.4] - 2026-10-06

{% if context.mobile and context.pwa and not context.web %}- Generate Expo web dependencies, SQLite WASM assets and single-page output for the mobile PWA host. Build and check the worker in the exported site.
{% endif %}{% if context.mobile %}- Set the Android Gradle dev-server address to loopback and give QA emulators four CPU cores.
- Apply the top safe-area inset to tab scenes and the sign-in header.
- Build the iOS simulator app in Release mode so Maestro has an embedded JavaScript bundle.
- Generate service worker build and check scripts for Expo web.
{% endif %}{% if context.mobile and context.auth_oidc %}- Store OIDC sessions in browser storage on web and SecureStore on native platforms.
{% endif %}{% if context.auth_oidc %}- Test expired access tokens separately from valid refresh sessions without waiting for expiry.
{% endif %}
- Keep the 0.7.2 changes under their release heading.

## [0.7.3] - 2026-10-05

{% if context.auth_oidc %}- Reconcile confidential backend clients without browser URLs. Preserve creation and rotated secrets.
- Retain failed identity deletion jobs for repair when cleaning terminal jobs.
{% endif %}{% if context.mobile %}- Reserve compact navigation height for the font scale and safe-area bottom inset.
{% endif %}{% if context.web or context.mobile %}- Document 48 dp Android navigation targets alongside the web and iOS minimums.
{% endif %}{% if context.quality_strict %}- Accept Markdown links to paths with balanced parentheses, including Expo route groups, and angle-bracket targets.
{% endif %}{% if context.mobile %}- Stop Android QA setup when the device probe fails or times out.
{% endif %}{% if context.worker %}- Use `job_kind` to query worker metrics separately from the Prometheus scrape job.
{% endif %}{% if context.mobile and context.auth_oidc %}- Use SecureStore-safe OIDC keys without a product key rewrite.
{% endif %}{% if context.mobile %}- Add expo-system-ui 57.0.4 to support automatic mobile appearance with Expo 57.
{% endif %}{% if context.web %}- Store Playwright browsers in web/.playwright-browsers so dependency reinstalls preserve them. Exclude the cache from Docker contexts.
- Bind the web preview server to 127.0.0.1 so Playwright readiness uses the same address.
{% endif %}
## [0.7.2] - 2026-10-05

{% if context.quality_strict and context.backend %}
- Create the LCOV output directory when Cargo uses an external target directory.
{% endif %}{% if context.mobile %}
- Pin Android command-line tools to build 13114758. Recreate an AVD when its API level, image tag, or architecture changes.
- Bound ADB startup and shutdown probes and the emulator boot wait.
- Remove Expo dependency check exclusions. Keep Jest and its types on major 29.
- Scroll native smoke flows to the consent button before tapping it.
{% if context.auth_oidc %}- Use a valid SecureStore key for the local-data registry so authenticated local data can open.
{% endif %}{% endif %}{% if context.web or context.mobile %}
- Pass navigation labels from English and German catalogs.
{% if context.mobile %}- Prepare Chrome before OIDC QA flows so its first-run screen does not block sign-in.
- Replace Android emulator config keys with spaces around `=` without leaving duplicate entries.
{% if context.auth_oidc %}- Return mobile OIDC callbacks to the app root after sign-in.
{% endif %}{% endif %}{% endif %}
## [0.7.1] - 2026-10-04

{% if context.web %}- Center generated web content beside the navigation rail. Keep action links at least 44 pixels high.
- Check initial dialog focus by accessible name. Support labeled inputs and buttons named by visible text.

{% endif %}{% if context.mobile %}- Match React and the react-dom override to Expo 57.0.26 bundled version 19.2.3. Keep React in Expo dependency checks.
{% endif %}

## [0.7.0] - 2026-10-04

- Install Corepack 0.36.0 before using pnpm in CI. Node 26 does not bundle Corepack.
- Update pnpm to 12.9.1. Fresh web and mobile lockfiles now pass frozen installation with linked Baukit packages.
{% if context.mobile %}- Match Jest types to the Jest 29 runtime.
{% endif %}{% if context.web or context.mobile %}- Update ESLint to 10.12 and Node types to 26.6.4.
{% endif %}{% if context.web %}- Update TanStack Query to 5.104.1.
{% endif %}{% if context.backend %}- Update Tokio to 1.53.2 and UUID to 1.27.
{% endif %}
{% if context.backend %}- Pin the development database to PostgreSQL `18.6-alpine`. Mount its volume
  at `/var/lib/postgresql` and recreate local volumes on major-version changes.
{% endif %}- Generated `AGENTS.md` is a regular file, with the same guidance as `CLAUDE.md`, for Turbo workspace discovery.
{% if context.mobile %}- Aligned both mobile templates on `expo-constants` 57.0.20. Pinned `react-dom` to 19.2.8 alongside mobile React and `@react-native/metro-config` to React Native 0.86.3, allowed the ESLint 10 peer for the accessibility plugin, and disabled the `core-js` postinstall through pnpm 12 `allowBuilds`.
{% endif %}{% if context.backend and context.auth_oidc %}- Moved the development Keycloak image to 26.8.0. The theme browser matrix covers 26.7.5 and 26.8.0.
- Moved the development Redis image to 8.10.2 Alpine.
{% endif %}

## [0.6.0] - 2026-10-02

{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
- Added error-response rules to the generated OpenAPI document. Every operation now documents the 400, {% if context.auth_oidc %}401, {% endif %}404, 413, 415, 422, {% if context.auth_oidc %}429, {% endif %}500, and 504 responses its middleware can return, with `X-Request-Id` on every response.
- Changed the backend to utoipa 6, which the Baukit OpenAPI helpers now require. Code that builds utoipa values by hand must wrap operation parameters and response headers in `RefOr::T`, and `HeaderBuilder::schema` takes an `Option`.
- Changed `backend/Dockerfile` to build on `rust:1.99.0-trixie` and run on `gcr.io/distroless/cc-debian13:nonroot`. Builder and runtime moved to Debian 13 together, so the binaries and the runtime share one glibc.
{% if context.quality_strict %}- Added `quality.openapi_compatibility` (`off`, `report`, or `enforce`) to the strict quality gate. It compares `backend/openapi.json` with the base revision and reads accepted breaks from `docs/openapi-accepted-breaks.json`.
{% endif %}{% endif %}{% if context.mobile %}- Added mobile tests for the theme mode control, record store seams, and the route heading focus hook, with `@testing-library/react-native` and `test-renderer` as dev dependencies.
{% if context.auth_oidc %}- Changed the auth mobile app to use the base Jest config, so `test:coverage` now enforces the 70% statement, branch, function, and line floors. The app also gains the `setup` and `test:coverage` scripts that the generated CI already calls. New tests cover the OIDC auth hook, the authenticated local-data provider, and the authenticated API runtime.
{% endif %}{% endif %}{% if context.web and context.auth_oidc %}- Added `E2E_WEB_PORT` to the web Keycloak stack test. Its global setup adds the chosen origin to the realm's web client through the admin API when the client lacks it, and `E2E_KEYCLOAK_WEB_CLIENT_ID` names that client.
- Changed the web Keycloak stack test to import `keycloakStack`, `createKeycloakTestUser`, `signInWithKeycloak`, and `allowKeycloakWebOrigin` from `@baukit/auth-node/keycloak-testing`, now a web dev dependency. `e2e/stack/keycloak.ts` keeps only this product's defaults and exports them as `stack`; the helpers take `stack` explicitly and no longer take a Playwright request context.
- Added auth web tests for the OIDC client wiring, the API parsers and default transport, the authenticated API runtime, and the local-data hook, so `test:coverage` passes its 70% floors.
{% endif %}{% if context.backend and context.auth_oidc %}- Added a Redis service to `compose.yaml` on `127.0.0.1:{{ context.redis_host_port }}`. The API's rate limiter needs it at startup, and `make dev` now starts it.
{% endif %}{% if context.mobile %}- Moved the native workflow to `gradle/actions/setup-gradle@v6` with `cache-provider: basic`, which keeps the MIT-licensed cache instead of v6's default proprietary one.
{% endif %}{% if context.web or context.mobile %}- Moved linting to ESLint 10, `@eslint/js` 10, and `typescript-eslint` 8.71. TypeScript stays on 6.0 because `typescript-eslint` does not accept TypeScript 7 yet.
{% endif %}{% if context.web or context.mobile %}- Changed `eslint.config.js` to `defineConfig` from `eslint/config`, since `tseslint.config` is deprecated, and pinned `@types/node` to 24.19 to match the Node 24 runtime.
{% endif %}{% if context.web %}- Moved the web app to React 19.3, Vite 8.3, Vitest 5, `@vitejs/plugin-react` 6.1, Playwright 1.63, and jsdom 30.1.
{% endif %}{% if context.mobile %}- Moved the mobile app to the current Expo SDK 57 patch releases (`expo` 57.0.26, `react-native` 0.86.3, `expo-router` 57.0.24, `jest-expo` 57.0.5), i18next 26.4, and react-i18next 17.0. `eslint-plugin-react-native-a11y` still declares ESLint 8 as its peer, so `pnpm-workspace.yaml` allows ESLint 10 for it and `@eslint/compat` wraps its rules. `test-renderer` stays on 1.2.0 because 1.3.0 needs React 19.3, which React Native 0.86 does not support.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
- Moved the generated GitHub workflows to `actions/checkout@v7`, `actions/cache@v6`, `actions/setup-node@v7`, `actions/setup-java@v6`, `actions/upload-artifact@v7`, and `dorny/paths-filter@v4`.
