# Observability stack base

This Flux-native base installs a deliberately non-HA observability stack in the
`observability` namespace:

- kube-prometheus-stack 88.2.0: Prometheus Operator, one Prometheus, one
  Alertmanager, Grafana, kube-state-metrics, and node-exporter;
- Loki 18.7.6 in monolithic mode with one filesystem-backed replica;
- Tempo 2.2.3 in monolithic mode with one filesystem-backed replica and OTLP
  gRPC/HTTP receivers;
- Alloy 1.11.1 as a DaemonSet that sends Kubernetes pod logs to Loki; and
- the existing `deploy/observability` chart as dashboards, recording rules, and
  alert content.

The starting retention budgets are visible once in each component's
HelmRelease: 30 days for metrics, 14 days for logs, and 7 days for traces. PVCs
use the cluster's default StorageClass so this base works with K3s `local-path`
and with an overlay-selected CSI default. Requests and limits are modest
single-node starting points; the local overlay may shrink them after measuring
the complete stack.

## Overlay contract

An overlay **must** provide all of the following:

1. A `source.toolkit.fluxcd.io/v1` `GitRepository` named `baukit` in
   `flux-system`. It must point at this repository at the release-train tag the
   cluster has selected. The `baukit-observability` HelmRelease resolves
   `./deploy/observability` from that source.
2. A Secret named `grafana-admin` in `observability`, with `admin-user` and
   `admin-password` keys. Grafana anonymous access, ingress, TLS, and any
   external URL are also overlay decisions; this base enables no ingress.
3. A real Alertmanager receiver before production traffic is served. Patch
   `kube-prometheus-stack.spec.values.alertmanager.config`: add an
   identity-bearing receiver, route actionable alerts to it, and route
   `BaukitDeadMansSwitch` to the external heartbeat integration. The base's
   `null` receiver intentionally drops every notification.
4. Storage sizing and a StorageClass appropriate to the node. The defaults are
   30 Gi for Prometheus, 20 Gi for Loki, 10 Gi for Tempo, and 1 Gi for
   Alertmanager. The overlay owns backup/recovery expectations for these
node-local PVCs.

The Loki and Tempo HelmReleases depend on `kube-prometheus-stack` because both
emit ServiceMonitors. This ordering ensures the Prometheus Operator CRDs exist
before Helm submits those resources; Alloy then depends on Loki, and the Baukit
content release depends on the Prometheus stack.
5. A ConfigMap named `observability-cluster` in `observability`, with an
   `environment` key whose value is the cluster's bounded environment identity
   (`local`, `testing`, or `production`). Alloy applies it to every log stream.

No domain, e-mail address, token, credential, bucket, endpoint, or user identity
is stored in this base.

## Signal ownership

There is one owner per signal. Alloy collects **logs only** from the K3s
container log files and writes them to Loki. Prometheus discovers
ServiceMonitors/PodMonitors and scrapes metrics directly. Applications send
traces directly to Tempo using
`http://tempo.observability.svc.cluster.local:4317` (OTLP gRPC) or port `4318`
(OTLP HTTP). Do not point application metrics or traces at the DaemonSet.

The Alloy pipeline promotes only the bounded Loki labels allowed by the Baukit
telemetry contract: `service`, `environment`, `namespace`, and `level`.
`service` is derived from Baukit workload labels, `environment` comes from the
overlay-owned ConfigMap, and deployed JSON logs supply `level`. Trace IDs and
request IDs remain fields in the log body.

An overlay may later add a separate Alloy or OpenTelemetry **gateway** for OTLP
normalization, batching, filtering, or tail sampling. If it does, it must update
application OTLP endpoints and keep that gateway as the sole trace collector;
the logs-only DaemonSet remains unchanged.

## Object-storage switch

Loki and Tempo deliberately start on filesystem storage. Moving either to S3
is an overlay operation because bucket names, endpoints, regions, and
credentials carry deployment identity. Supply credentials through SOPS-managed
Secrets and environment references, never inline values.

For Loki, patch `loki.storage`, add the credential Secret through
`singleBinary.extraEnvFrom`, enable environment expansion, and append a
future-dated `loki.schemaConfig.configs` period whose `object_store` is `s3`.
Never rewrite a schema period that has already stored data. For Tempo, patch
`tempo.storage.trace` from `backend: local` to `backend: s3`, reference the
credential Secret through `tempo.extraEnvFrom`, and preserve the 168-hour
retention budget unless an explicit capacity review changes it.

This base deliberately leaves out Mimir, Thanos, HA replicas, remote storage,
public ingress, authentication policy, notification integrations, synthetic
checks, and an OTLP gateway.

## Validation

From the repository root:

```sh
kustomize build deploy/platform/observability-stack
python3 deploy/observability/lint/check-metric-names.py
helm lint deploy/observability
```

The platform validation harness additionally templates each pinned upstream
chart with the inline HelmRelease values and validates the built base with
`kubeconform -strict -ignore-missing-schemas`.
