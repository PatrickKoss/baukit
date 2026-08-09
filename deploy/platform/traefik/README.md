# Traefik

This base installs Traefik chart `41.2.0` (Traefik `v3.7.10`) as one DaemonSet
pod per node in the `traefik` namespace. Container hostPorts 80 and 443 make
the single node the edge without `hostNetwork` or a cloud LoadBalancer. The
Service is `ClusterIP`. The API/dashboard is disabled; ping on the internal
`traefik` entrypoint and Prometheus metrics on internal port 9100 have no
hostPort.

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
CIDRs if a proxy is later introduced, and any `priorityClassName` override.

No dashboard route, ops route, Certificate, domain, credential, or Secret is
created here.
