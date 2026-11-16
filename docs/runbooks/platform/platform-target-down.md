# BaukitPlatformTargetDown

## Meaning

Prometheus failed to scrape a target in a namespace labelled
`app.kubernetes.io/part-of=baukit-platform` for 10 minutes. The alert names `job`, `instance` and
`namespace`. Platform components are Flux, cert-manager, CNPG, Traefik, Kyverno, Falco and the
observability stack itself.

## Impact

The component behind the target either is down or cannot be monitored. Alerts that depend on its
metrics are blind until the scrape works again.

## First checks

All failing platform targets:

```promql
up == 0 and on (namespace) kube_namespace_labels{label_app_kubernetes_io_part_of="baukit-platform"}
```

The pod behind `instance`:

```sh
kubectl -n <namespace> get pods -o wide | grep <instance-ip>
kubectl -n <namespace> describe pod <pod>
kubectl -n <namespace> logs <pod> --previous --tail=100
```

The scrape error:

```sh
kubectl -n observability port-forward svc/kube-prometheus-stack-prometheus 9090:9090 &
curl -s 'http://127.0.0.1:9090/api/v1/targets?state=active' \
  | jq '.data.activeTargets[] | select(.health=="down") | {job: .labels.job, scrapeUrl, lastError}'
```

Default-deny policies in platform namespaces must admit Prometheus:

```sh
kubectl -n <namespace> get networkpolicy
```

Flux state of the release that owns the component:

```sh
flux get helmreleases -A --status-selector ready=false
```

## Fix

- The pod crashes: fix it from its previous logs, or roll back the HelmRelease
  (`flux suspend hr <name> -n <namespace>`, revert in Git, `flux resume hr <name> -n <namespace>`).
- `lastError` is a timeout or connection refused while the pod is healthy: add a NetworkPolicy
  that admits ingress from the `observability` namespace on the metrics port.
- The metrics port moved after a chart upgrade: update the ServiceMonitor or PodMonitor port name.

## Escalation

Platform on-call. If the target is Prometheus, Alertmanager or the operator, treat it as critical:
the alert path itself is at risk.
