# Observability stack base

This Flux-native base installs a deliberately non-HA observability stack in the
`observability` namespace, which enforces Pod Security `restricted`. Node-exporter
and Alloy need host access, so they run in the privileged `observability-agents`
namespace:

- kube-prometheus-stack 93.3.0: Prometheus Operator, one Prometheus, one
  Alertmanager, Grafana, kube-state-metrics, and node-exporter;
- Loki 21.2.3 in monolithic mode with one filesystem-backed replica;
- Tempo 3.2.1 (Tempo 3.1.1) in monolithic mode with one filesystem-backed replica and OTLP
  gRPC/HTTP receivers;
- Alloy 1.16.0 as a DaemonSet that sends Kubernetes pod logs to Loki;
- the existing `deploy/observability` chart as dashboards, recording rules, and
  alert content; and
- the `deploy/observability-platform` chart with alerts, recording rules and
  dashboards for the platform components. Its `runbookBaseUrl` value sets the
  prefix of every alert's `runbook_url`.

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
   cluster has selected. The `baukit-observability` and
   `baukit-platform-observability` HelmReleases resolve `./deploy/observability`
   and `./deploy/observability-platform` from that source.
2. A Secret named `grafana-admin` in `observability`, with `admin-user` and
   `admin-password` keys. Grafana anonymous access, ingress, TLS, and any
   external URL are also overlay decisions; this base enables no ingress.
3. A Secret named `alertmanager-routing` in `observability` with an
   `alertmanager.yaml` key. Start from `alertmanager/alertmanager.example.yaml`:
   routes by `severity` and `component`, Watchdog first to an external
   heartbeat receiver, and the two inhibit rules (critical inhibits warning
   for the same `alertname` and `namespace`, and `InfoInhibitor`). The base
   mounts this Secret into the Alertmanager pod, so without it the pod stays
   in `ContainerCreating` with `secret "alertmanager-routing" not found`, no
   Watchdog reaches the heartbeat, and the external dead man's switch fires.
   The mount matters because Prometheus Operator (seen with v0.93.1) does not fail on a
   missing config Secret: it logs `config secret not found, using default
   Alertmanager configuration` and starts with a config that drops every
   alert. `lint/check_alertmanager_config.py` checks the example and every
   overlay Secret. A webhook URL on `*.svc.cluster.local` must name a Service
   that the same overlay renders. The local overlay routes every receiver to
   `alert-sink` in `observability`, which logs each notification as one JSON
   line on stdout, so local alerts land in Loki, and serves the last 500 on
   `GET /alerts`.
4. Storage sizing and a StorageClass appropriate to the node. The defaults are
   30 Gi for Prometheus, 20 Gi for Loki, 10 Gi for Tempo, and 1 Gi for
   Alertmanager. The overlay owns backup/recovery expectations for these
node-local PVCs.

The Loki and Tempo HelmReleases depend on `kube-prometheus-stack` because both
emit ServiceMonitors. This ordering ensures the Prometheus Operator CRDs exist
before Helm submits those resources; Alloy then depends on Loki, and the Baukit
content release depends on the Prometheus stack.
5. A ConfigMap named `observability-cluster` in `observability-agents`, with an
   `environment` key whose value is the cluster's bounded environment identity
   (`local`, `testing`, or `production`). Alloy applies it to every log stream.

No domain, e-mail address, token, credential, bucket, endpoint, or user identity
is stored in this base.

## Priority

Every pod in this base runs as `platform-standard`: Prometheus, Alertmanager,
the operator and its admission Jobs, Grafana, kube-state-metrics,
node-exporter, Alloy, Loki and Tempo. The base admits kured to Prometheus on
9090 for its pre-reboot alert check.

## Alert routing

Alertmanager is the single sink for three sources:

- Prometheus rules from kube-prometheus-stack and both baukit packs.
- Loki ruler rules from `deploy/observability-platform/loki-rules/`. The pack
  renders them into ConfigMap `baukit-platform-observability-loki-rules`, which
  Loki mounts as the rule directory of tenant `fake`. Loki does not start until
  that ConfigMap exists.
- Flux events. `flux-notifications.yaml` holds a `Provider` of type
  `alertmanager` and an `Alert` for error events from every Kustomization,
  GitRepository and HelmRelease namespace. Flux sends them with
  `severity="error"` and `component="flux"`. The lifecycle installs Flux's
  notification-controller (`platform-up.sh` adds it to `--components`;
  `flux bootstrap` installs it by default).

## Storage limits

Every store has a time retention and a cap sized to its PVC. An overlay that
changes a PVC size changes the matching cap in the same patch.

Prometheus deletes blocks by size once `retentionSize` is reached, so it never
fills its PVC. `retentionSize` is about 85% of the PVC in Prometheus base-2
units: `25GB` for the 30 Gi base, `1740MB` for the 2 Gi local overlay.
`deploy/platform/lint/check_retention_size.py` builds every base and overlay in
`validate.sh` and fails when `retentionSize` is missing or above 90% of the PVC.
`BaukitPrometheusStorageNearRetentionSize` fires at 90% of `retentionSize`.

Loki and Tempo have no size-based retention. Their caps are time retention plus
bounded working space. `KubePersistentVolumeFillingUp` from the
kube-prometheus-stack defaults fires as a warning above 85% used when the volume
will fill within four days, and as critical above 97% used.
`deploy/observability-platform/tests/kubernetes-storage.test.yml` tests that
upstream rule, and `deploy/platform/lint/check_upstream_rules.py` fails when the
rendered chart's rule drifts from the copy the test loads.

Loki deletes chunks older than `retention_period` (336h base, 48h local)
through the compactor: `retention_enabled: true`, delete requests in the object
store, and the working directory `/var/loki/compactor` on the PVC. The 20 Gi
base fits 14 days while stored chunks and index average below 17 GiB / 14 days
= 1.2 GiB per day, about 14 KiB/s after compression. The local overlay writes
chunks to RustFS, so its 2 Gi Loki PVC holds only the WAL, the TSDB index cache
and the compactor working directory. Its 48 hours of chunks share the RustFS
PVC with the Barman backups.

Tempo deletes blocks older than `block_retention` (168h base, 48h local). The
live-store cuts a block at `live_store.max_block_bytes` (256 MiB base, 64 MiB
local) and the backend worker stops growing a block at
`backend_worker.compaction.max_block_bytes` (1 GiB base, 200 MiB local), so a
compaction needs at most twice that as transient space. On the 10 Gi base that
leaves 10 GiB - 2 x 1 GiB - 2 x 256 MiB = 7.5 GiB for blocks, which fits 7 days
below 1.07 GiB per day (about 12.7 KiB/s). On the 2 Gi local PVC it leaves
2048 - 400 - 128 = 1520 MiB, which fits 48 hours below 760 MiB per day (about
9 KiB/s). `local-footprint.md` records no log or trace ingest rates, so these
are budgets, not measurements.

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

The publishable Loki and Tempo bases start on filesystem storage. Moving either
to S3 is an overlay operation because bucket names, endpoints, regions, and
credentials carry deployment identity. Supply credentials through SOPS-managed
Secrets and environment references, never inline values.

The local Loki overlay uses RustFS 1.0.1, bucket `baukit-local`, region
`us-east-1`, and path-style addressing. Runtime credentials come from
`rustfs-root` in `observability` through environment expansion. This schema is
for a fresh disposable cluster. Recreate the integration harness when switching
its storage; persistent overlays must preserve existing schema periods.

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
