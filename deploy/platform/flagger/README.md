# Flagger progressive-delivery base

This base installs Flagger 1.44.0 in `flagger-system` with the direct Traefik
provider, the shared kube-prometheus-stack server, and
`baukit.dev/workload` as the only rollout selector. The Flagger chart installs
its CRDs through Helm. Six reusable Baukit MetricTemplates live beside the
controller; products supply only `product`, `environment`, intervals, and
threshold ranges on their Canary resources.

## Overlay contract

Compose this base only after Traefik and kube-prometheus-stack are available.
The target cluster must provide:

- the `traefik.io/v1alpha1` CRDs and a Traefik controller watching the app
  namespaces;
- Prometheus at
  `kube-prometheus-stack-prometheus.observability.svc.cluster.local:9090`,
  selecting ServiceMonitors from the app and `flagger-system` namespaces;
- API Deployments whose immutable selector and pod template both carry a
  unique `baukit.dev/workload=<product>-api` label;
- an IngressRoute that points to a same-name TraefikService which Flagger may
  adopt, plus clone-safe Services, ServiceMonitors, and NetworkPolicies;
- one product-owned Canary in each environment. Reference the shared
  MetricTemplates in `flagger-system`; API error percentage and HTTP p95 are
  the normal blocking metrics. Worker templates are dashboard/diagnostic
  building blocks unless the product explicitly makes workers promotable.

Overlay the HelmRelease only when the Prometheus address or resource sizing
differs. Do not change `meshProvider` or add Gateway API resources without a
separate platform decision.

## Per-app loadtester pattern

The loadtester intentionally is not installed by this cluster-scoped base.
The blocking command is product code and its NetworkPolicy trust boundary is
the app namespace, so each app installs the pinned Flagger loadtester chart
`0.38.0` in its own environment namespace. Name the HelmRelease
`flagger-loadtester`, reuse this base's `HelmRepository/flagger` by creating a
namespaced repository or reference an allowed shared source, and set:

```yaml
values:
  fullnameOverride: flagger-loadtester
  image:
    repository: <product-rollout-gate-image>
    tag: <same-immutable-release-pin>
    pullPolicy: Never # local k3d flow; use the platform registry policy elsewhere
  podLabels:
    baukit.dev/product: <product>
    baukit.dev/role: rollout-gate
  cmd:
    timeout: 2m
```

Build the custom image FROM a small runtime, copy in the upstream
`/home/app/loadtester` binary from
`ghcr.io/fluxcd/flagger-loadtester:0.38.0`, and add the pinned product smoke
executable. The Canary `pre-rollout` webhook uses `type: cmd` and invokes that
executable against `<product>-canary` directly. Credentials remain in an
existing SOPS-managed Secret, never in this base or image. This shape keeps
the zero-customer-traffic gate headless and lets each app grant only DNS,
Keycloak, Prometheus, and canary-Service egress.

