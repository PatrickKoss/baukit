# 30. Consumer friction in tooling and test support

Plan item 4. Six small corrections that products patch around today. Each one
lands as its own commit with its own test and changelog line.

## Source revisions

Product files were re-read on 2026-09-27 at Eigenruhe `f74cebb`, Hebkit
`841bf5d`, Redemut `a782538`, Runtime Analyzer `d47bfd5`, Solo Leveling System
`3461eaf`, and Tiefgang `2d37a06`. Eigenruhe moved past the plan's `e44ff88`;
its lint script did not change in a way that affects this note. Baukit baseline
is `ef6ac1a`.

## 1. Metric-name linter takes product input

### Observed repeated glue

`deploy/observability/lint/check-metric-names.py` hardcoded its root and metric
list. Every product that lints dashboards loads the file as a module and patches
its globals:

- `/home/patrick/projects/redemut/scripts/observability-lint.py:38-56` patches
  `ROOT` and `SPEC_METRICS`, then builds a temporary symlink tree because its
  dashboards live in `deploy/observability/grafana/dashboards` and its alerts in
  a single `deploy/observability/alerts.yml`.
- `/home/patrick/projects/tiefgang/infra/observability-lint.py:47-50` patches
  `ROOT`, `OBSERVABILITY` (`infra/grafana`), and `SPEC_METRICS`.
- `/home/patrick/projects/eigenruhe/scripts/observability-lint.py:128-133`
  patches the same globals, then runs its own dashboard coverage audit.
- `/home/patrick/projects/runtime-analyzer/deploy/observability/lint/check-metric-names.py:31-45`
  is an edited vendored copy that adds seven product names to `SPEC_METRICS`
  and lists `job_duration_seconds` a second time in `HISTOGRAM_METRICS`.
- The generated `docs/observability-lint.md` documented the module-global patch
  as the contract.

### Baukit owner and public contract

`deploy/observability/lint/check-metric-names.py` gains three arguments:

- `--observability-root DIR`: directory holding `dashboards/`, `alerts/`, and
  `recording-rules/`. Defaults to Baukit's own `deploy/observability`.
- `--allowlist FILE`: one product metric per line, `#` comments, and an optional
  `histogram` marker that also allows `_bucket`, `_count`, and `_sum`.
- `--rules FILE`: extra rule file outside the root, repeatable. Redemut's
  single `alerts.yml` needs it.

`main(argv)` takes the argument list. Called with no arguments, as CI and
`verify/verify-observability.sh` do, it lints Baukit's pack and prints the same
summary line as before. Paths in messages are relative to the Baukit checkout
when inside it, otherwise relative to the working directory.

### Failure behavior

Exit 0 on success and 1 on lint problems, as before. Exit 2 when the root is not
a directory, the allowlist cannot be read, or an allowlist line has an invalid
name, an unknown marker, or a duplicate name. A missing root used to pass with
zero files, which hid a typo in a product path.

### Privacy boundary

None. The linter reads only local dashboard and rule files.

### Supported runtimes

Python 3.9 or later, standard library only.

### Tests

`deploy/observability/lint/test_check_metric_names.py` covers the unchanged
Baukit invocation, a product root with an allowlist and histogram series,
missing allowlist entries, the histogram marker, `--rules`, invalid allowlist
entries, and a missing root. CI runs it next to the linter. The new arguments
were also run by hand against Redemut, Tiefgang, and Runtime Analyzer at the
revisions above; all three passed with an allowlist built from their current
product names.

### Template change

The generated CI job and the strict `scripts/quality-gate.sh` now look for
`deploy/observability/product-metrics.txt` instead of
`scripts/observability-lint.py` and call the linter with
`--observability-root deploy/observability --allowlist
deploy/observability/product-metrics.txt`. The generated
`docs/observability-lint.md` documents the allowlist instead of the shim.

### Breaks

- A generated product's CI no longer runs `scripts/observability-lint.py`. A
  product that relies on its shim must add
  `deploy/observability/product-metrics.txt` and adjust the arguments in its CI
  job to its layout.
- Shims that set module globals and call `linter.main()` still work, because
  the defaults read those globals at call time. That path is no longer
  documented.

### Product adoption

- Redemut: delete `scripts/observability-lint.py`; call the linter with
  `--observability-root deploy/observability/grafana --rules
  deploy/observability/alerts.yml --allowlist <file>` and mark
  `redemut_sync_batch_size` as `histogram`.
- Tiefgang: delete `infra/observability-lint.py`; call the linter with
  `--observability-root infra/grafana --allowlist <file>`.
- Eigenruhe: keep the product coverage audit in `scripts/observability-lint.py`,
  but replace the global patching at `:128-133` with a call to
  `linter.main(["--observability-root", "infra/grafana", "--allowlist", ...])`
  or write the allowlist from `product_metrics.rs` first.
- Runtime Analyzer: delete the vendored
  `deploy/observability/lint/check-metric-names.py`, move its seven product
  names to an allowlist with `job_duration_seconds histogram`, and run Baukit's
  copy with `--observability-root deploy/observability`.
