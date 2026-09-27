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

[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts"]
```

`critical_paths` contains Playwright spec paths relative to `web/`, such as `e2e/tests/checkout.spec.ts`. The strict runner executes those specs on desktop and mobile WebKit projects with `--repeat-each`. Keep this list short and reserve it for flows where intermittent WebKit failures would block a release.

`full_stack_e2e` stays false until the product has an external-service harness. When enabled, the repository must provide `scripts/full-stack-e2e.sh`. That product-owned script starts services, loads deterministic fixtures, runs the tests, and cleans up.

List each committed OpenAPI TypeScript declaration in `openapi.consumers`. `scripts/openapi-client.sh` regenerates the entire list. The strict gate fails when a listed file is uncommitted or changes after regeneration.

After the schema diff, the strict gate runs `backend/tests/openapi_drift.rs`. Besides the drift check, that test fails on any property or path or query parameter name that is not camelCase and prints the JSON pointer of each one. Names a standard defines, such as OAuth 2.0 `access_token`, go in the test's `STANDARD_DEFINED_NAMES` list.

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

Observability checks run only when `deploy/observability/product-metrics.txt` exists. A production image builds only when `backend/Dockerfile` exists. The generated web app sets `capabilities.pwa = false`. If a product adds a PWA and changes that value to true, its web package must provide `build:sw:check`; the strict runner calls it in both local and CI runs.

## Migration

Existing strict products can copy `scripts/check-markdown-links.py` and its test from the current template. Add both commands to `scripts/quality-gate.sh` before capability-specific build checks:

```sh
python3 scripts/check-markdown-links.test.py
python3 scripts/check-markdown-links.py README.md CLAUDE.md AGENTS.md docs
```

Pass different repository-relative roots if the product keeps Markdown elsewhere. Commit the files before checking them because the script uses `git ls-files` to keep local and CI input identical.
