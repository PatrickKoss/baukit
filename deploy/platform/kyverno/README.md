# Kyverno

This base installs Kyverno chart `3.10.0` (Kyverno `v1.20.0`) and Policy Reporter chart `3.11.2` in
the `kyverno` namespace. Policies live in the separate `kyverno-policies` base.

Kyverno 1.20 no longer executes the legacy `kyverno.io` policy types (`ClusterPolicy`, `Policy`,
legacy `PolicyException`). The chart refuses to install or upgrade while such objects exist, so the
old `policies.yaml` is gone and its three policies now live in `kyverno-policies` as
`ValidatingPolicy` objects. Flux prunes the old `ClusterPolicy` objects before the upgrade runs.

Settings that matter:

- one replica per controller, each with a PodDisruptionBudget of `maxUnavailable: 1`. A single
  replica still drains, and a multi-replica overlay keeps all but one pod up.
- ServiceMonitors for the admission, background, cleanup and reports controllers, plus Policy
  Reporter's ServiceMonitor. Policy Reporter runs with metrics on, UI, Kyverno plugin and Grafana
  dashboards off.
- `PolicyException` objects are accepted only in the `kyverno` namespace, which tenants cannot write.
- the admission webhook timeout is 5 seconds (`--webhookTimeout=5`).
- webhooks skip `kube-system`, `flux-system` and `kyverno`, so a broken policy cannot block the
  controllers that would repair it.

Install this base before `kyverno-policies`; the policies need the CRDs it brings.
