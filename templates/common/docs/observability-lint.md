# Observability metric-name lint

Dashboards, alerts, and recording rules reference metric names as strings.
Nothing in a compiler catches a renamed metric, so a panel keeps rendering an
empty graph and an alert keeps not firing. Baukit ships a linter that compares
every name referenced by an observability file against the set of names that are
supposed to exist.

The linter lives in Baukit, at
`deploy/observability/lint/check-metric-names.py`. It knows Baukit's own metric
vocabulary. The product tells it where its dashboards live and which metric
names it adds, through command-line arguments.

## The allowlist

Write `deploy/observability/product-metrics.txt`. The `observability-lint` job
in `.github/workflows/ci.yml` looks for exactly that path: when the file is
absent the job reports that this product declares no dashboards and passes; when
it is present the job clones Baukit at the matching tag and runs the linter
against `deploy/observability`.

```text
# One product metric per line. Baukit's own metric names are always allowed.
{{ context.app_crate }}_items_created_total
# "histogram" also allows the _bucket, _count, and _sum series.
{{ context.app_crate }}_items_request_duration_seconds histogram
```

Keep dashboards in `deploy/observability/dashboards/*.json` and Prometheus rules
in `deploy/observability/alerts/*.yml` or
`deploy/observability/recording-rules/*.yml`. An empty allowlist is valid for a
product that only charts Baukit metrics.

## The arguments

| Argument | Meaning |
|---|---|
| `--observability-root DIR` | Directory holding `dashboards/`, `alerts/`, and `recording-rules/`. |
| `--allowlist FILE` | Product metric names, one per line, with an optional `histogram` marker and `#` comments. |
| `--rules FILE` | Extra rule file outside the root. Repeat for each file. |

The linter exits 0 on success, 1 when an expression references an unknown
metric or breaks a naming rule, and 2 when the root is missing or the allowlist
has an invalid, duplicate, or malformed entry. A product with another layout
changes the arguments in the CI job and in `scripts/quality-gate.sh` when it
has one.

Run it locally the same way CI does:

```sh
git clone --branch v{{ context.template_version }} --depth 1 \
  https://github.com/PatrickKoss/baukit.git /tmp/baukit
python3 /tmp/baukit/deploy/observability/lint/check-metric-names.py \
  --observability-root deploy/observability \
  --allowlist deploy/observability/product-metrics.txt
```
