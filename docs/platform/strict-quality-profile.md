# Strict quality profile

`baukit new --quality strict` adds a blocking CI job and `scripts/quality-gate.sh`. The script runs the same checks locally in the same order. The standard profile remains the default.

Every strict project also gets `scripts/check-markdown-links.py`. The strict gate runs its standard-library test suite, then checks committed Markdown under `README.md`, `CLAUDE.md`, `AGENTS.md`, and `docs/`. Relative links resolve from the source file. Repository-absolute links resolve from the product root. External URLs, query strings, and fragments do not trigger network requests. The checker reports the source file, line, and missing target. It does not validate anchors.

The generator reads capabilities before it renders the workflow. A backend gets coverage, MSRV, migration, OpenAPI, and production-image checks. A web app gets unit coverage plus the complete Chromium and WebKit Playwright suite, including geometry and console-warning specs. A mobile app gets Expo Doctor, lint, type checks, Jest coverage, an iOS JavaScript bundle, and Android `assembleDebug`. Missing capabilities do not leave placeholder jobs.

## Manifest settings

The root manifest owns the strict thresholds and product declarations:

```toml
[quality]
profile = "strict"
backend_coverage_lines = 70
critical_paths = []
webkit_repeats = 3
full_stack_e2e = false
openapi_compatibility = "off"
android_emulator_port = 5644
android_emulators = 2
shared = []

[quality.gate_budget_minutes]
full = 10
affected = 5

[quality.affected]
"backend/crates/**" = ["backend"]
"web/src/**" = ["web"]
"web/e2e/**" = ["web-e2e"]
"mobile/**" = ["mobile"]
"mobile/.maestro/**" = ["mobile", "android-build", "android-e2e"]
"docs/**" = []

[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts"]
```

`critical_paths` contains Playwright spec paths relative to `web/`, such as `e2e/tests/checkout.spec.ts`. The strict runner executes those specs on desktop and mobile WebKit projects with `--repeat-each`. Keep this list short and reserve it for flows where intermittent WebKit failures would block a release.

`full_stack_e2e` stays false until the product has an external-service harness. When enabled, the repository must provide `scripts/full-stack-e2e.sh`. That product-owned script starts services, loads deterministic fixtures, runs the tests, and cleans up.

List each committed OpenAPI TypeScript declaration in `openapi.consumers`. `scripts/openapi-client.sh` regenerates the entire list. The strict gate fails when a listed file is uncommitted or changes after regeneration.

nextest runs `backend/tests/openapi_drift.rs` with the rest of the backend suite. Besides the drift check, that test fails on any property or path or query parameter name that is not camelCase and prints the JSON pointer of each one. Names a standard defines, such as OAuth 2.0 `access_token`, go in the test's `STANDARD_DEFINED_NAMES` list.

`android_emulator_port`, `android_emulators` and `android_wifi` exist only for mobile products. `android_wifi`, false by default, gives each emulator its own Wi-Fi network for flows that toggle Wi-Fi. `baukit new` picks the first port of a block of 8 from the product name and sets 2 emulators, which use the port and the next even port. The gate shards Maestro flows across them. `gate_budget_minutes` is per repository and both values are settable: 10 and 5 minutes for small products (the default for new ones), 20 and 10 for mid-size ones, 30 and 15 for large ones. `gate_budget_minutes`, `affected` and `shared` exist only in strict products. `[quality.resources]` maps a resource name to the lanes or steps that lock it exclusively while `BAUKIT_GATE_LOCK_DIR` is set, and `[quality.shared_resources]` does the same for steps that share it with each other (see `docs/quality-gate.md`). The generator writes an `affected` table that fits the product's capabilities. It names `android-build` and `android-e2e` for `mobile/.maestro/**`, so a Maestro-only change runs the flows even when a product moves the Android steps out of the `mobile` lane. `baukit doctor` prints the emulator ports the settings take and rejects an odd emulator port, one outside 5556 to 5810, an emulator count outside 1 to 4, a range that runs past its block of 8 ports (see the port table in the generated `docs/quality-gate.md`), a zero budget, absolute globs, globs with `..`, and lane or step names the product's gate doesn't have.

## Fast gate

The gate runs lanes in parallel, splits `BAUKIT_GATE_CORES` across them, runs each suite once, and prints a timing table and budget check at the end. `--affected <base>` runs the fast checks plus the steps that `quality.affected` selects. The product copy of [the gate guide](../../templates/common/__strict__/docs/quality-gate.md) lists the lanes, dependencies, caches, Playwright tags, emulator health checks and selection rules.

## OpenAPI compatibility

`quality.openapi_compatibility` compares `openapi.schema` with its copy at the base revision and lists every change that can break a client. It needs the strict profile and a backend; `baukit doctor` rejects it otherwise.

| Value | Effect |
|---|---|
| `off` | The default. The gate skips the comparison. |
| `report` | The gate prints each break and keeps going. Use it until the product has live clients. |
| `enforce` | The gate fails on any break that is not accepted. |

The gate runs Baukit's `scripts/check-openapi-compatibility.py` right after the drift test, with the same base revision as the migration guard. A path-sourced Baukit runs the script from that checkout. Otherwise the gate clones the Baukit tag the product was generated from, so a product needs a Baukit release that ships the script. When the base revision has no schema yet, the script prints `skipped` and passes. You can also run it directly:

```sh
python3 path/to/baukit/scripts/check-openapi-compatibility.py \
  --base-revision origin/main --current backend/openapi.json --path-prefix /v1/
```

Each finding is one line: a rule ID, a location such as `GET /v1/items response 200 application/json $[].tags`, and a message. The rules cover removed operations, parameters, responses, and media types; changed `operationId` or security; parameters and request bodies that became required; new success statuses; and schema changes. For schemas, the script knows which side of the wire it is on. A request schema breaks when it rejects a value it used to accept: a narrower enum or constraint, a newly required property, closed `additionalProperties`, or a changed default. A response schema breaks when it may return a value an old client does not expect: a wider enum or constraint, or a property that is no longer required. Removed properties, changed types, formats, and patterns break on both sides. The script follows local `$ref`s and handles recursive schemas.

### False positives

The script reports a change when a client that conformed to the base document could fail or lose data against the current one. When it cannot decide, such as a reordered `oneOf`, it reports the change too. A missed break reaches clients; a false alarm costs one line in a file. Additive changes pass: new operations, optional parameters, optional request properties, new response properties, and new error statuses. Adding an enum value to a response counts as a break, because a strict client may reject it.

### Accepted breaks

Record an intentional break in `docs/openapi-accepted-breaks.json`. The gate passes the file to the script when it exists:

```json
{
  "accepted": [
    {
      "rule": "property-removed",
      "location": "GET /v1/items response 200 application/json $[].legacyName",
      "reason": "No client reads legacyName; removed before launch."
    }
  ]
}
```

Copy `rule` and `location` from the report. Every entry needs a non-empty reason, and an unknown rule fails the run. An entry that no longer matches prints a note, so delete it once the base revision contains the change.

## Local use

Install the tools required by the enabled capabilities, then run:

```sh
sh scripts/quality-gate.sh
```

For migration checks, set `BAUKIT_BASE_REVISION` to the pull request base commit. Without it, the script uses the merge base with `origin/main`, then `HEAD^`, then the first commit.

The backend coverage gate uses `cargo llvm-cov nextest --run-ignored all`. Docker must be running because generated PostgreSQL tests use `#[ignore]`. Coverage HTML and LCOV files are written under `backend/target/llvm-cov/`, and CI uploads both.

Observability checks run only when `deploy/observability/product-metrics.txt` exists. A production image builds only when `backend/Dockerfile` exists. Generated products set `capabilities.pwa = false`. If a product adds a PWA and changes that value to true, the app that serves it must provide `build:sw:check`. That is the web package when the product has a web app, and otherwise the mobile package, whose Expo web export serves the worker. The strict runner calls it in both local and CI runs.

## Migration

### v0.17.1

Mobile products take these steps:

1. Copy `mobile/scripts/qa/android-adb.py` from a v0.17.1 render.
2. Copy `mobile/scripts/qa/android-env.sh` from the render. A product that changed its copy ports the snapshot block instead: compute `snapshot_name` with `android-adb.py --snapshot-name`, load a snapshot only when its `<avd>.avd/<snapshot>.ready` marker exists, hold `<avd>.avd/baukit-snapshot.lock` around the cold boot and save, close fd 9 in the emulator subshell, run `--settle` and the health check before `logcat -b all -c` and the save, and then `--prune-snapshots`. Replace `-gpu swiftshader` with `-gpu swiftshader_indirect`.
3. Set `BAUKIT_QA_SETTLE_TIMEOUT_SECONDS` only when a host needs more than the 600-second default.
4. Run `make e2e-android` twice, once to save the new snapshot and once to load it. The old `baukit-clean` snapshot goes away with the first save.

### v0.17.0

A product's `.T` task takes these steps:

1. Copy `scripts/quality-gate.sh`, `scripts/gate.py`, `scripts/gate.test.py` and `docs/quality-gate.md` from a v0.17.0 render, then add `/.quality-gate/` to `.gitignore`.
2. Add `[quality.gate_budget_minutes]` with the repository's tier (10/5 small, 20/10 mid, 30/15 large), `[quality.affected]` and `shared = []` to `baukit.toml`. Start from the table a fresh `baukit new --quality strict` writes for the same capabilities, then add globs for product directories. Paths that match no glob run the full gate, so a missing glob costs time, not coverage. Run `baukit doctor`.
3. Mobile products set `quality.android_emulator_port` and `quality.android_emulators = 2`, and check that the second port is free on the host. Use the value `baukit new` would write, or keep the port the product already uses. Replace `BAUKIT_QA_ANDROID_EMULATOR_PORT` with `BAUKIT_QA_ANDROID_PORT` in scripts and docs.
4. Make `ci` in the Makefile run `sh scripts/quality-gate.sh` alone. Remove `check` or other suites that run before the gate.
5. Delete clamps that read `Committed_AS` or cap test threads, such as `scripts/cargo-build-jobs.py`, and let `BAUKIT_GATE_CORES` drive the counts. Remove `--runInBand` from `mobile/package.json`. Tests that share state go in a nextest test group or a separate Jest project.
6. Tag Playwright tests that check only application logic with `@logic`. Leave `qa-*.spec.ts` untagged.
7. Copy `android-env.sh`, `android-adb.py`, `run-maestro.sh` and `build-android.sh` from the template, plus the `ci` target and the `PLAYWRIGHT_BROWSERS_PATH ?=` line in the Makefile, and `preflight.sh`.
8. Run the full gate once and compare `.quality-gate/timings.txt` with the old gate. Then merge through `--affected`.

### Markdown links

Existing strict products can copy `scripts/check-markdown-links.py` and its test from the current template. Add both commands to `scripts/quality-gate.sh` before capability-specific build checks:

```sh
python3 scripts/check-markdown-links.test.py
python3 scripts/check-markdown-links.py README.md CLAUDE.md AGENTS.md docs
```

Pass different repository-relative roots if the product keeps Markdown elsewhere. Commit the files before checking them because the script uses `git ls-files` to keep local and CI input identical.
