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
| `helmrelease-drift-detection` | `driftDetection.mode` is `enabled` |
| `helmrelease-crd-policy` | charts with a Helm `crds/` directory use `CreateReplace` on install and upgrade |
| `helmrelease-helm-tests` | charts that render Helm test hooks set `test.enable: true` |
| `chart-source-interval` | HelmRepository and OCIRepository `spec.interval` is 1h or more |
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

## In-cluster policies offline

`run_cluster_policies.py` runs from `validate.sh` after the Flux conventions. It runs
`kyverno test` over `../../kyverno-policies/tests/`, then `kyverno apply` once per rendered base,
overlay and HelmRelease chart, and once per `baukit-app` profile: the chart rendered with
`deploy/chart/baukit-app/tests/all-options-values.yaml` and with
`kyverno-policies/tests/app-profiles/generated-platform-values.yaml`. Each apply uses every
Kyverno policy the rendered bases install (the baukit set and the upstream Pod Security chart)
plus `policies/` here.

The stage reads its cluster state from the render, not from a cluster:

- `PolicyException` objects come from the rendered bases, so the gate sees exactly what Flux
  applies from `kyverno-policies/exceptions/`.
- Namespace labels come from the rendered Namespaces. The `baukit-app` profiles land in a
  `product` namespace labelled like `components/tenant`, plus `baukit.dev/environment:
  production`.
- `resource.Get` and `resource.List` see the overlay's `kyverno/baukit-registry-allowlist`
  ConfigMap and every rendered Ingress.
- Chart objects without a namespace get the HelmRelease's target namespace.

A `fail` fails the gate when its policy rejects at admission (`Deny`) or runs only here (`CI`).
A `fail` from an `Audit` policy is listed in the report and does not fail the gate, because the
cluster would only report it. Every `error` fails the gate, and so does a run with no results.
The per-policy table, with each finding, lands in `.ci-gate/platform-kyverno-policies.txt`.

`generated-platform-values.yaml` is the `deploy/values.yaml` that `baukit new` writes for the
`platform` snapshot flavor. A test compares its sha256 with `cli/tests/snapshots/platform.tree`,
so a template change that re-blesses the snapshot also needs a fresh copy.
