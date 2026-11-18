# Kyverno

This base installs Kyverno chart `3.10.0` (Kyverno `v1.20.0`) in the `kyverno` namespace. Policies
live in the separate `kyverno-policies` base, and the ServiceMonitors and Policy Reporter chart `3.11.2`
in `kyverno-monitoring`.

Kyverno 1.20 no longer executes the legacy `kyverno.io` policy types (`ClusterPolicy`, `Policy`,
legacy `PolicyException`). The chart refuses to install or upgrade while such objects exist, so the
old `policies.yaml` is gone and its three policies now live in `kyverno-policies` as
`ValidatingPolicy` objects. Flux prunes the old `ClusterPolicy` objects before the upgrade runs.

Settings that matter:

- one replica per controller, each with a PodDisruptionBudget of `maxUnavailable: 1`. A single
  replica still drains, and a multi-replica overlay keeps all but one pod up.
- the admission controller runs as `platform-critical`, the other controllers and Policy Reporter
  as `platform-standard`. The chart has no priority key for its hook Jobs and test Pods, so a
  `postRenderers` patch sets `platform-standard` on them.
- no ServiceMonitors on first install. The release reads the optional `kyverno-monitoring-values`
  ConfigMap, which `kyverno-monitoring` ships to turn on the ServiceMonitors for the admission,
  background, cleanup and reports controllers.
- `PolicyException` objects are accepted only in the `kyverno` namespace, which tenants cannot write.
- the admission webhook timeout is 5 seconds (`--webhookTimeout=5`).
- the admission and reports controllers may open HTTPS connections to the internet. Both fetch
  image signatures for `verify-baukit-images`, at admission and in background scans.
- webhooks skip `kube-system`, `flux-system` and `kyverno`, so a broken policy cannot block the
  controllers that would repair it.

Install this base right after the foundation and before `kyverno-policies`, which needs the CRDs it
brings. Every workload except the foundation comes after `kyverno-policies`, so Kyverno admits it.
Kyverno itself must not wait for the monitoring CRDs, which is why the monitoring parts live in
`kyverno-monitoring`, ordered after both `kyverno-policies` and the observability stack.
