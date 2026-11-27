# Run the quality gate

A generated product ships one gate that runs the same checks on your machine and in CI. This page
explains the standard and strict profiles, how to run the strict gate locally, what each lane
does, and how to read a failure. At the end you can run the gate, narrow it to changed paths, and
tune it through `baukit.toml`.

## Pick a profile

`baukit new --quality standard|strict` picks the profile and writes it to `quality.profile`.

The standard profile renders one GitHub Actions job per concern in `.github/workflows/ci.yml`:
`generated-scripts`, `repository-lint`, `baukit-doctor`, `backend`, `backend-msrv`, `api-drift`,
`docker-build`, `web`, `web-coverage`, `e2e-web`, `mobile`, `mobile-coverage` and
`observability-lint`, each present only when its capability is. Locally, `make check` runs
preflight, format, lint, tests with `--include-ignored`, the script tests, and the web and mobile
checks.

The strict profile renders two jobs, `generated-scripts` and `strict-quality`. The second runs
`sh scripts/quality-gate.sh`, and so does `make ci`. The generator reads capabilities first: a
backend gets coverage, MSRV, migration, OpenAPI and production-image checks, a web app gets unit
coverage plus the Chromium and WebKit Playwright suites, and a mobile app gets Expo Doctor, lint,
type checks, Jest coverage, an iOS JavaScript bundle and Android `assembleDebug`. A missing
capability leaves no placeholder job.

[ADR 0014](../adr/0014-one-quality-gate.md) records why one script owns the gate.

## Run the strict gate

Install the tools the enabled capabilities need. `mise install` reads `mise.toml`, which pins
Rust, the linters, the scanners and cosign. Docker must run, because the PostgreSQL tests are
`#[ignore]` tests and the gate runs them. Then:

```sh
sh scripts/quality-gate.sh
```

For the migration guard and the OpenAPI comparison, set `BAUKIT_BASE_REVISION` to the pull
request base. Without it the gate uses the merge base with `origin/main`, then `HEAD^`, then the
first commit.

Useful flags:

```sh
sh scripts/quality-gate.sh --list-steps          # name lane deps always|selected tags
sh scripts/quality-gate.sh --plan                # print the selection, run nothing
sh scripts/quality-gate.sh --affected origin/main
sh scripts/quality-gate.sh --step rust-clippy
```

Each lane writes its log and the timing table under the gate's output directory. The product copy
of the gate guide, `docs/quality-gate.md`, rendered from
`templates/common/__strict__/docs/quality-gate.md`, lists every step and cache.

On a clean tree the gate skips a step that already passed on the same inputs and prints
`cached (passed at <sha>, <age>)`. Passes are kept for `[quality.cache] max_age_days`, 7 by
default. List steps that read data from outside the repository, such as `secret-scan` and `osv`,
under `[quality.cache] never`. The cache is off in CI, with `--no-cache` and with
`BAUKIT_GATE_CACHE=off`. `baukit doctor` requires `scripts/gate_results.py` and its test.

## Lanes

Lanes run in parallel and split `BAUKIT_GATE_CORES` between them. A sample from a strict product
with a backend and web app:

| Lane | Steps |
| --- | --- |
| fast | openapi, rust-fmt, migrations, scripts-tests, markdown-links, preflight, baukit-doctor, baukit-check, workflow-lint, secret-scan, osv, cargo-deny, deploy-values, python-lint, shell-lint, metric-names, spec-purposes |
| backend | rust-clippy, rust-msrv, rust-tests, rust-doctests |
| fuzz | the fuzz targets |
| images | the production image builds |
| web | web-install, web-build, web-lint, web-unit, web-service-worker, playwright, webkit-repeats |

A mobile product adds the mobile and Android lanes. `--list-steps` on your product is the real
list. `baukit doctor` fails when it lacks a template step for a declared capability.

`baukit-check` runs every `baukit check` rule. The [CLI reference](cli.md#baukit-check) lists
them.

## Tune the gate

The keys live in `baukit.toml`. The [manifest reference](manifest.md#quality) gives types and
defaults.

- `[quality.affected]` maps globs to the lanes or steps `--affected` selects. `quality.shared`
  lists paths that force the full gate.
- `[quality.gate_budget_minutes]` sets the `full` and `affected` budgets. Going over prints a
  warning.
- `[gate.worker-caps]` caps a worker variable such as `E2E_WORKERS`.
- `quality.light` marks lanes that take no core share. `[quality.resources]` and its shared
  and creator variants lock host resources across concurrent gates when `BAUKIT_GATE_LOCK_DIR`
  is set.
- `[quality.disk]` sets the free-disk floor the gate checks before it starts.

Product-only steps go in `scripts/quality-gate.local.sh`, which defines `local_steps` and a
`step_<name>` function per step. The template never overwrites that file.

## Coverage and thresholds

The backend coverage step runs `cargo llvm-cov nextest --run-ignored all` and fails below
`quality.backend_coverage_lines`. It writes HTML and LCOV under `backend/target/llvm-cov/`, and CI
uploads both. `quality.critical_paths` names Playwright specs the gate repeats
`quality.webkit_repeats` times on WebKit. `quality.full_stack_e2e = true` adds
`scripts/full-stack-e2e.sh`.

Observability checks run only when `deploy/observability/product-metrics.txt` exists. A
production image builds only when `backend/Dockerfile` exists. With `capabilities.pwa = true`, the
app that serves the worker, web or else mobile, must provide `build:sw:check`.

The `markdown-links` step runs `scripts/check-markdown-links.py` over the product's Markdown and
fails on a relative link to a missing file or heading.

## OpenAPI compatibility

`quality.openapi_compatibility` compares `openapi.schema` with its copy at the base revision and
lists every change that can break a client. It needs the strict profile and a backend.

| Value | Effect |
| --- | --- |
| `off` | Default. The gate skips the comparison. |
| `report` | The gate prints each break and continues. Use it until the product has live clients. |
| `enforce` | The gate fails on any break that is not accepted. |

The gate runs baukit's `scripts/check-openapi-compatibility.py` after the drift test. A path
dependency runs it from that checkout. Otherwise the gate clones the baukit tag the product was
generated from. When the base revision has no schema yet, the script prints `skipped` and passes.
To run it directly:

```sh
python3 path/to/baukit/scripts/check-openapi-compatibility.py \
  --base-revision origin/main --current backend/openapi.json --path-prefix /v1/
```

Each finding is a rule ID, a location such as
`GET /v1/items response 200 application/json $[].tags`, and a message. A request schema breaks
when it rejects a value it used to accept. A response schema breaks when it may return a value an
old client does not expect, so a new response enum value counts. Removed properties and changed
types break both ways. When the script cannot decide, as with a reordered `oneOf`, it reports.
Additive changes pass.

Record an intentional break in `docs/openapi-accepted-breaks.json`:

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

Copy `rule` and `location` from the report. Each entry needs a reason, and an unknown rule fails
the run. A stale entry prints a note. Delete it once the base revision has the change.

## Read a failure

Start at the first failed step in the summary, then open its lane log. A red step is never
unrelated until you have read its output. Fix the cause, then rerun only that step with
`--step NAME`, and finish with `--affected` against the base. Do not lower a threshold, add a
suppression without a reason, or raise a `baukit-check.toml` limit to get green. `baukit check`
rejects most of those. The `baukit-quality-gates` agent skill walks through common failures.

## Reference

- `templates/common/__strict__/docs/quality-gate.md`
- `templates/common/__strict__/scripts/gate.py`
- [`docs/platform/native-quality-gates.md`](../platform/native-quality-gates.md)
- [`lint/README.md`](../../lint/README.md)
- [The baukit CLI](cli.md)
- [`baukit.toml` reference](manifest.md)
- [ADR 0014: one quality gate](../adr/0014-one-quality-gate.md)
- [ADR 0027: verification strategy](../adr/0027-verification-strategy.md)
- [ADR 0013: code rules enforced by baukit check](../adr/0013-code-rules-enforced-by-baukit-check.md)
