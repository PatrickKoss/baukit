# Baukit application chart

`baukit-app` is the reusable workload chart for a Baukit product. A product pins this chart as a Helm dependency and keeps only thin environment-specific values files. It creates one API Deployment, an optional worker Deployment, a pre-release migration Job, and an opt-in seed Job. Each process has its own runtime image.

The migration hook only starts the migrate process. Advisory locking, lock timeout, and expand/migrate/contract compatibility remain application responsibilities; the API never runs migrations implicitly.

## Use as a dependency

```yaml
# Chart.yaml in a product repository
dependencies:
  - name: baukit-app
    version: 0.1.0
    repository: oci://ghcr.io/patrickkoss/charts
    alias: application
```

Place product settings under the alias in the wrapper's `values.yaml`. The repository example at [`../examples/minimal-api-values.yaml`](../examples/minimal-api-values.yaml) can also be rendered directly:

```sh
helm template minimal-api . -f ../examples/minimal-api-values.yaml
```

## Runtime contract

For `product: minimal-api`, the chart normalizes the configuration prefix to `MINIMAL_API`. It injects `MINIMAL_API_ENVIRONMENT`, `OTEL_EXPORTER_OTLP_ENDPOINT` when configured, listener/drain values such as `MINIMAL_API__HTTP__PORT`, and every entry in `config.overrides` as `MINIMAL_API__<KEY>`. Override keys are suffixes, so `DATABASE__MAX_CONNECTIONS` becomes `MINIMAL_API__DATABASE__MAX_CONNECTIONS`.

Secrets are only consumed through `envFrom` references to existing Secrets. This chart deliberately has no value that accepts secret contents; create encrypted Secret manifests separately with SOPS/age.

Kubernetes metadata carries `app.kubernetes.io/part-of` and `baukit.dev/product` for the telemetry `product` identity, plus `app.kubernetes.io/component` and `baukit.dev/process` for `api`, `worker`, `migrate`, or `seed`. The application continues to own OpenTelemetry resource attributes such as `service.name=<product>-<process>` and its build version/commit.

## Networking and scraping

Only the API Service is an Ingress backend. The private ops Service exposes the ops container port inside the cluster and is never referenced by an Ingress. Liveness uses `/healthz`, readiness uses `/readyz`, and Prometheus uses `/metrics`.

Annotation discovery is enabled on ops Services by default. Set `opsService.prometheusScrape.enabled=false` (and the worker equivalent) if the collector does not use annotations. `serviceMonitor.enabled=true` offers an alternative for Prometheus Operator installations; the template is emitted only when the cluster advertises `monitoring.coreos.com/v1/ServiceMonitor`. The chart ships no CRDs.

NetworkPolicy is enabled by default and selects every process in the release. It denies all ingress and egress, then allows the configured Traefik pods to the API port, configured Prometheus pods to ops ports, DNS to CoreDNS, and the explicit `networkPolicy.additionalEgress` rules. The defaults assume K3s Traefik in `kube-system`, Prometheus in `observability`, and CoreDNS in `kube-system`; products must adjust selectors and add database, OTLP, identity-provider, and external-service destinations. The example shows PostgreSQL and Alloy rules.

## Values

### Identity, security, and configuration

| Value | Default | Description |
|---|---:|---|
| `nameOverride` | `""` | Override the chart name used in resource names and labels. |
| `fullnameOverride` | `""` | Override the complete release-scoped resource-name prefix. |
| `product` | `minimal-api` | Stable product identity used in labels and as the default config prefix. |
| `deploymentEnvironment` | `local` | `local`, `staging`, or `production`; injected as `<APP>_ENVIRONMENT`. |
| `imagePullSecrets` | `[]` | Pod-level existing image-pull Secret references, typically for GHCR. |
| `podLabels` | `{}` | Extra labels added to every process pod. Do not overwrite selector labels. |
| `podAnnotations` | `{}` | Extra annotations added to every process pod. |
| `podSecurityContext.runAsNonRoot` | `true` | Require all process containers to run as a non-root user. |
| `podSecurityContext.seccompProfile.type` | `RuntimeDefault` | Pod seccomp profile. |
| `containerSecurityContext.allowPrivilegeEscalation` | `false` | Prevent privilege escalation. |
| `containerSecurityContext.readOnlyRootFilesystem` | `true` | Make each process root filesystem read-only. |
| `containerSecurityContext.capabilities.drop` | `[ALL]` | Linux capabilities dropped from every process. |
| `serviceAccount.automountToken` | `false` | Automount the namespace's default service-account token. |
| `config.envPrefix` | `""` | Explicit application config prefix; empty derives it from `product`. |
| `config.otlpEndpoint` | `""` | Value for `OTEL_EXPORTER_OTLP_ENDPOINT`; omitted when empty. |
| `config.overrides` | `{}` | Non-secret config suffix/value map rendered with the `<APP>__` prefix. |
| `config.existingSecretRefs` | `[]` | `envFrom.secretRef` objects with required `name` and optional `optional`. |

The pod and container security-context maps can be replaced when an image needs a numeric `runAsUser`, writable volume mounts, or another seccomp policy; weakening them should be a deliberate product decision.

### API

| Value | Default | Description |
|---|---:|---|
| `api.replicas` | `1` | API replicas when HPA is disabled. |
| `api.image.repository` | `ghcr.io/example/minimal-api` | API-only image repository. |
| `api.image.tag` | `latest` | API image tag. Pin an immutable release tag or digest policy in production. |
| `api.image.pullPolicy` | `IfNotPresent` | API image pull policy. |
| `api.command` / `api.args` | `[]` / `[]` | Optional container entrypoint and arguments. |
| `api.ports.http` / `api.ports.ops` | `8080` / `9090` | Public API and private operations listener ports. |
| `api.terminationGracePeriodSeconds` | `30` | SIGTERM drain window; also injected as `SHUTDOWN__DRAIN_TIMEOUT`. |
| `api.strategy.maxUnavailable` / `api.strategy.maxSurge` | `0` / `1` | RollingUpdate availability controls. |
| `api.resources.requests.cpu` / `memory` | `50m` / `64Mi` | Initial API resource requests. |
| `api.resources.limits.cpu` / `memory` | `250m` / `128Mi` | Initial API resource limits. |
| `api.livenessProbe.enabled` | `true` | Enable ops `/healthz` liveness probing. |
| `api.livenessProbe.initialDelaySeconds` / `periodSeconds` / `timeoutSeconds` / `failureThreshold` | `5` / `10` / `2` / `3` | Liveness timing and failure threshold. |
| `api.readinessProbe.enabled` | `true` | Enable ops `/readyz` readiness probing. |
| `api.readinessProbe.initialDelaySeconds` / `periodSeconds` / `timeoutSeconds` / `failureThreshold` | `2` / `5` / `2` / `3` | Readiness timing and failure threshold. |
| `api.nodeSelector` / `api.affinity` / `api.tolerations` | `{}` / `{}` / `[]` | API scheduling controls. |

### Worker and release Jobs

| Value | Default | Description |
|---|---:|---|
| `worker.enabled` / `worker.replicas` | `false` / `1` | Enable the optional worker Deployment and set replicas. |
| `worker.image.repository` / `tag` / `pullPolicy` | `ghcr.io/example/minimal-worker` / `latest` / `IfNotPresent` | Worker-only image settings. |
| `worker.command` / `worker.args` | `[]` / `[]` | Optional worker entrypoint and arguments. |
| `worker.ops.enabled` / `worker.ops.port` | `true` / `9090` | Enable the worker ops listener, probes, and private ops Service. |
| `worker.terminationGracePeriodSeconds` | `30` | Worker SIGTERM drain window and injected shutdown timeout. |
| `worker.resources.requests.cpu` / `memory` | `50m` / `64Mi` | Initial worker requests. |
| `worker.resources.limits.cpu` / `memory` | `250m` / `128Mi` | Initial worker limits. |
| `worker.livenessProbe.enabled` | `true` | Probe worker `/healthz` when worker ops is enabled. |
| `worker.livenessProbe.initialDelaySeconds` / `periodSeconds` / `timeoutSeconds` / `failureThreshold` | `5` / `10` / `2` / `3` | Worker liveness settings. |
| `worker.readinessProbe.enabled` | `true` | Probe worker `/readyz` when worker ops is enabled. |
| `worker.readinessProbe.initialDelaySeconds` / `periodSeconds` / `timeoutSeconds` / `failureThreshold` | `2` / `5` / `2` / `3` | Worker readiness settings. |
| `worker.nodeSelector` / `worker.affinity` / `worker.tolerations` | `{}` / `{}` / `[]` | Worker scheduling controls. |
| `migration.enabled` | `true` | Create the `pre-install,pre-upgrade` migration hook Job. |
| `migration.image.repository` / `tag` / `pullPolicy` | `ghcr.io/example/minimal-migrate` / `latest` / `IfNotPresent` | Migrator-only image settings. |
| `migration.command` / `migration.args` | `[/app/migrate]` / `[]` | Migrator entrypoint and arguments. |
| `migration.backoffLimit` / `migration.activeDeadlineSeconds` | `2` / `600` | Migration retry and deadline bounds. |
| `migration.resources.requests.cpu` / `memory` | `50m` / `64Mi` | Migration requests. |
| `migration.resources.limits.cpu` / `memory` | `250m` / `128Mi` | Migration limits. |
| `seed.enabled` / `seed.allowProduction` | `false` / `false` | Enable post-release seeding; production also requires an explicit safety override. |
| `seed.image.repository` / `tag` / `pullPolicy` | `ghcr.io/example/minimal-seed` / `latest` / `IfNotPresent` | Seed-only image settings. |
| `seed.command` / `seed.args` | `[/app/seed]` / `[]` | Seed entrypoint and arguments. |
| `seed.backoffLimit` / `seed.activeDeadlineSeconds` | `1` / `600` | Seed retry and deadline bounds. |
| `seed.resources.requests.cpu` / `memory` | `50m` / `64Mi` | Seed requests. |
| `seed.resources.limits.cpu` / `memory` | `250m` / `128Mi` | Seed limits. |

Migration uses `helm.sh/hook-delete-policy: before-hook-creation`. Seed uses a `post-install,post-upgrade` hook and refuses to render in production unless both seed flags are explicitly true.

### Services, ingress, and scaling

| Value | Default | Description |
|---|---:|---|
| `service.type` / `service.port` | `ClusterIP` / `80` | API-only Service type and port. |
| `service.annotations` | `{}` | Extra annotations on the public API Service. |
| `opsService.type` / `opsService.port` | `ClusterIP` / `9090` | Private API ops Service type and port. |
| `opsService.annotations` | `{}` | Extra API ops Service annotations. |
| `opsService.prometheusScrape.enabled` / `path` | `true` / `/metrics` | Add correctly ported `prometheus.io/*` scrape annotations. |
| `workerOpsService.type` / `workerOpsService.port` | `ClusterIP` / `9090` | Private worker ops Service settings. |
| `workerOpsService.annotations` | `{}` | Extra worker ops Service annotations. |
| `workerOpsService.prometheusScrape.enabled` / `path` | `true` / `/metrics` | Worker annotation-discovery settings. |
| `ingress.enabled` / `ingress.className` | `false` / `traefik` | Create the API-only Ingress and choose its class. |
| `ingress.annotations` | `{}` | Additional API Ingress annotations. |
| `ingress.certManager.clusterIssuer` | `""` | Add the cert-manager cluster-issuer annotation when set. |
| `ingress.rateLimitMiddleware` | `""` | Add Traefik's router middleware annotation when set; empty keeps the hook disabled. |
| `ingress.hosts` | example host/path | Networking v1 host entries, each with `paths[].path` and `paths[].pathType`. |
| `ingress.tls` | `[]` | Standard Ingress TLS entries with `secretName` and `hosts`. |
| `autoscaling.enabled` | `false` | Enable an autoscaling/v2 HPA for the API. |
| `autoscaling.minReplicas` / `maxReplicas` | `1` / `4` | HPA replica range. |
| `autoscaling.targetCPUUtilizationPercentage` | `70` | CPU utilization target; set empty to omit. |
| `autoscaling.targetMemoryUtilizationPercentage` | `""` | Optional memory utilization target. |
| `serviceMonitor.enabled` | `false` | Request capability-gated API/worker ServiceMonitors. |
| `serviceMonitor.interval` / `scrapeTimeout` | `30s` / `10s` | ServiceMonitor endpoint timing. |
| `serviceMonitor.labels` | `{}` | Extra labels used by the Prometheus Operator's monitor selector. |

### NetworkPolicy

| Value | Default | Description |
|---|---:|---|
| `networkPolicy.enabled` | `true` | Create default-deny and explicit allow policies. |
| `networkPolicy.ingressController.namespaceSelector` | `kube-system` | Namespace selector for the ingress controller. |
| `networkPolicy.ingressController.podSelector` | `app.kubernetes.io/name=traefik` | Traefik pod selector allowed to the API port. |
| `networkPolicy.prometheus.namespaceSelector` | `observability` | Namespace selector for metrics scrapers. |
| `networkPolicy.prometheus.podSelector` | `app.kubernetes.io/name=prometheus` | Scraper pod selector allowed to ops ports. |
| `networkPolicy.dns.namespaceSelector` | `kube-system` | Namespace selector for DNS. |
| `networkPolicy.dns.podSelector` | `k8s-app=kube-dns` | CoreDNS pod selector. |
| `networkPolicy.additionalEgress` | `[]` | Raw NetworkPolicy egress-rule list appended after TCP/UDP DNS. |

## Local validation

```sh
helm lint .
helm template default .
helm template example . -f ../examples/minimal-api-values.yaml
```

