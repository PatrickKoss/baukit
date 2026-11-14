# Traefik

This base installs Traefik chart `41.6.1` (Traefik `v3.7.13`) as a Deployment
in the `traefik` namespace behind a `LoadBalancer` Service. On k3s, ServiceLB
(klipper-lb) answers that Service: its `svclb-traefik` pods in `kube-system`
hold host ports 80 and 443 and forward to the Traefik pods. Traefik itself uses
no hostPort and no `hostNetwork`, so the `traefik` namespace can run the
`restricted` Pod Security level, and a rolling update can start the surge pod
while the old pod still serves (`maxUnavailable: 0`, `maxSurge: 1`).

The base runs one replica. A multi-node overlay sets `deployment.replicas: 2`
so a drain or rollout never leaves the edge empty. The PodDisruptionBudget uses
`maxUnavailable: 1`: with one replica it never blocks a node drain, and with two
it keeps one pod serving. The pods run with `priorityClassName:
platform-critical` from the cluster-baseline base. Helm creates and replaces the
chart CRDs on install and upgrade (`crds: CreateReplace`).

The API/dashboard is disabled. Ping on the internal `traefik` entrypoint and
Prometheus metrics on internal port 9100 are not exposed by the Service.

Keep ServiceLB enabled on k3s (do not pass `--disable=servicelb`). A cluster
without ServiceLB needs another LoadBalancer implementation, for example the
Hetzner CCM with a managed Load Balancer.

Before applying this base, disable k3s's packaged Traefik on every server:

```yaml
# /etc/rancher/k3s/config.yaml
disable: [traefik]
```

Restart k3s after changing that file and remove any old packaged Traefik
resources before Flux reconciles this release.

The chart release ships `default-rate-limit` and `default-connection-limit`
Middleware objects and attaches them to both public entrypoints, so every route
gets the baseline limits. An overlay may patch the thresholds or replace the
entrypoint middleware list with a reviewed alternative. The objects are
rendered through the chart so Helm installs the Traefik CRDs before creating
them.

The chart emits a ServiceMonitor. The observability stack must install the
Prometheus Operator CRDs before the HelmRelease reconciles (typically by a Flux
`dependsOn` in the consuming overlay). The API check is disabled only so Helm
can render before that CRD exists; Kubernetes still requires the CRD at apply
time. The overlay supplies Ingresses, domains, TLS references, trusted proxy
CIDRs if a proxy is later introduced, and the replica count.

No dashboard route, ops route, Certificate, domain, credential, or Secret is
created here.
