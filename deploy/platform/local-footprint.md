# Local platform footprint

Measured on 2026-08-09 at approximately 11:13 CEST after the complete local
platform, the generated `fixture` API and worker, a PostgreSQL backup, and the
restore proof had reached steady state. The sample was taken with
`kubectl top pods --all-namespaces` and `kubectl top nodes` after a 30-second
idle period.

| Namespace | Component | CPU | Memory working set |
|---|---|---:|---:|
| cert-manager | cert-manager | 1m | 40 MiB |
| cert-manager | cainjector | 1m | 56 MiB |
| cert-manager | webhook | 1m | 24 MiB |
| cnpg-system | Barman Cloud plugin | 1m | 21 MiB |
| cnpg-system | CloudNativePG operator | 2m | 54 MiB |
| fixture | API | 1m | 6 MiB |
| fixture | worker | 2m | 6 MiB |
| flux-system | Helm controller | 1m | 113 MiB |
| flux-system | Kustomize controller | 1m | 153 MiB |
| flux-system | source controller | 1m | 111 MiB |
| keycloak | Keycloak | 3m | 645 MiB |
| keycloak | Keycloak operator | 5m | 261 MiB |
| kube-system | CoreDNS | 1m | 30 MiB |
| kube-system | local-path provisioner | 1m | 19 MiB |
| kube-system | metrics server | 3m | 26 MiB |
| observability | Alertmanager | 1m | 47 MiB |
| observability | Alloy | 5m | 108 MiB |
| observability | Grafana | 5m | 353 MiB |
| observability | kube-state-metrics | 1m | 31 MiB |
| observability | Prometheus operator | 2m | 33 MiB |
| observability | node exporter | 1m | 19 MiB |
| observability | Loki | 3m | 110 MiB |
| observability | Loki gateway | 1m | 31 MiB |
| observability | Prometheus | 10m | 694 MiB |
| observability | Tempo | 2m | 93 MiB |
| postgres | MinIO | 1m | 113 MiB |
| postgres | PostgreSQL plus Barman sidecar | 5m | 134 MiB |
| postgres | restore-test Pushgateway | 0m | 20 MiB |
| traefik | Traefik | 1m | 32 MiB |
| **Pod total** | **29 running pods** | **63m** | **3,383 MiB (3.30 GiB)** |

The k3d node reported **123m CPU and 6,452 MiB (6.30 GiB) memory** in total,
against 24 allocatable CPUs and 48,954,860 KiB (46.69 GiB) allocatable memory.
The difference from the pod total includes k3s/containerd, the Kubernetes node
processes, kernel working set, and other node-level accounting.

## Testing-node sizing implication

A testing node should not be sized at 8 GiB: this idle sample already consumed
6.30 GiB at node level and startup, reconciliation, restore, and test workloads
produce short-lived peaks. A **4 vCPU / 16 GiB** node is a reasonable minimum
for the single-node testing flavor, leaving roughly 2.5x idle-memory headroom;
use 8 vCPU / 16 GiB if faster simultaneous Helm reconciliation and restore
proofs are important. Re-measure on the intended Hetzner instance before
locking I3a sizing.

## Caveats

`kubectl top` reports cAdvisor memory working set, not strict process RSS, and
its point-in-time CPU sample can miss bursts. k3d also runs k3s inside a Docker
container on WSL2, so node memory includes a different kernel/container layer
than a real Hetzner VM. The host was deliberately unconstrained (24 CPUs and
46.69 GiB allocatable), so these numbers prove the idle footprint but not
behavior under CPU or memory pressure.
