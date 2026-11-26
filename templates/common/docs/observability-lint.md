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

The template does not generate `deploy/observability/product-metrics.txt`, so
the `observability-lint` job in `.github/workflows/ci.yml` skips the lint until
you add one. When the file is absent the job reports that this product declares
no dashboards and passes. When it is present the job clones Baukit at the
matching tag and runs the linter against `deploy/observability`. Write the file
like this:

```text
# One product metric per line. Baukit's own metric names are always allowed.
{{ context.app_crate }}_items_created_total
# "histogram" also allows the _bucket, _count, and _sum series.
{{ context.app_crate }}_items_request_duration_seconds histogram
```

A backend product gets dashboards in `deploy/observability/dashboards/*.json`
and Prometheus rules in `deploy/observability/rules/*.yml`. The linter reads
rules only from `alerts/` and `recording-rules/` under the root, so pass each
file in `rules/` with `--rules`, in the CI job and locally. An empty allowlist
is valid for a product that only charts Baukit metrics.

## The arguments

| Argument | Meaning |
|---|---|
| `--observability-root DIR` | Directory holding `dashboards/`, `alerts/`, and `recording-rules/`. |
| `--allowlist FILE` | Product metric names, one per line, with an optional `histogram` marker and `#` comments. |
| `--rules FILE` | Extra rule file that the `alerts/` and `recording-rules/` scan misses. Repeat for each file. |

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
  --allowlist deploy/observability/product-metrics.txt \
  --rules deploy/observability/rules/api.rules.yml
```
