# Flux convention policies

Kyverno `ValidatingPolicy` objects for Flux HelmRelease, HelmRepository, OCIRepository and
Kustomization objects. They run only offline, from `validate.sh`, and never load into a cluster.
Cluster policies live in `../../kyverno-policies/`.

| Policy | Rule |
|---|---|
| `helmrelease-exact-chart-version` | a HelmRepository chart pins one exact version |
| `helmrelease-remediation-retries` | `install.remediation.retries` and `upgrade.remediation.retries` are set |
| `helmrelease-cleanup-on-fail` | `upgrade.cleanupOnFail: true` |
| `helmrelease-timeout` | `spec.timeout` is set |
| `helmrelease-drift-detection` | `driftDetection.mode` is `warn` or `enabled` |
| `helmrelease-crd-policy` | charts with a Helm `crds/` directory use `CreateReplace` on install and upgrade |
| `helmrelease-helm-tests` | charts that render Helm test hooks set `test.enable: true` |
| `helmrepository-interval` | `spec.interval` is 1h or more |
| `ocirepository-exact-ref` | `spec.ref` is a digest or an exact version tag |
| `kustomization-prune` | `prune: true` |
| `kustomization-wait` | `wait: true` |
| `kustomization-dead-health-checks` | no `healthChecks` list next to `wait: true` |

There is no exception mechanism. Fix the base.

`chart-contents.toml` records, per pinned chart, whether it ships CRDs in a `crds/` directory
(in the chart or a subchart) and whether its render contains a `helm.sh/hook: test` object. The
`chartsWithCrds` and `chartsWithTests` lists in the two policies come from it. `run_ci_policies.py`
fails when the file or the lists drift from the pinned charts. After a chart bump, regenerate both:

```sh
deploy/platform/validate.sh --render-to /tmp/rendered
python3 deploy/platform/lint/ci-policies/chart_contents.py --rendered /tmp/rendered \
  --chart-cache deploy/platform/.helm-cache --write
```

Each policy has `tests/<name>/kyverno-test.yaml` with passing and failing resources:

```sh
kyverno test deploy/platform/lint/ci-policies/tests/
```
