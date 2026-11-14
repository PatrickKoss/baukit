# Platform observability pack

Helm chart `baukit-platform-observability`. It renders one PrometheusRule per file in `alerts/` and
one Grafana dashboard ConfigMap per file in `dashboards/`. The first dot-separated part of a file
name is its component, and `components.<name>` in `values.yaml` switches it on or off. Grafana's
sidecar puts each dashboard in the folder `<dashboards.folderRoot>/<component>`.

## Dashboards

| File | Component | Source |
|---|---|---|
| `platform.json` | platform | Baukit alerts by component, Flux Ready state, PSA and policy violations |
| `flux.json` | flux | baukit |
| `cert-manager.json` | cert-manager | baukit |
| `cnpg.json` | cnpg | vendored, see below |
| `traefik.json` | traefik | baukit |
| `kyverno.json` | kyverno | baukit, Kyverno and Policy Reporter |
| `falco.json` | falco | baukit |

Every query must use metrics listed in `metrics.txt`. `scripts/ci-gate.sh --step metric-names` runs
the metric lint and `deploy/observability/lint/check-dashboards.py`, which checks the dashboard
structure and that every datasource is a datasource variable or a Grafana built-in.

## Vendored CloudNativePG dashboard

`dashboards/cnpg.json` is the upstream CloudNativePG dashboard.

- Source: https://github.com/cloudnative-pg/grafana-dashboards/blob/cececeb393fb7c5400b4fa290aca68041293a127/charts/cluster/grafana-dashboard.json
  (tag `cluster-v0.0.5`).
- License: Apache-2.0.
- Upstream sha256: `676b10ae3a8850108d484e4491739207b2ad1ebca44300472168d81a8788e4fb`.

Changes from upstream, datasource references only. Hard-coded `prometheus` uids become
`${DS_PROMETHEUS}`, the annotation uid `grafana` becomes `-- Grafana --`, and `${DS_EXPRESSION}`
becomes Grafana's built-in expression uid `__expr__`. `__inputs` and `__requires` are dropped because
the sidecar does not resolve them. `uid` is `baukit-platform-cnpg` and `id` is null.

To update, download the new file at a pinned commit, apply the same changes, record the new commit
and sha256 here, and add any new metric names to `metrics.txt` after checking them against the
operator's `config/manager/default-monitoring.yaml` at the pinned operator version.
