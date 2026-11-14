# Changelog

## [Unreleased]

- Every HelmRelease sets a measured `timeout`, `upgrade.cleanupOnFail: true` and `driftDetection.mode: warn`. Charts with a Helm `crds/` directory (alloy, flagger, kube-prometheus-stack, loki, posthog, traefik) install and upgrade CRDs with `CreateReplace`. Falco and Kyverno run their Helm tests (`test.enable: true`). `validate.sh` enforces these rules, exact chart versions, HelmRepository intervals of 1h or more, and `prune`/`wait` on Flux Kustomizations with offline Kyverno policies in `lint/ci-policies/`.

- Breaking: run Traefik as a Deployment behind k3s ServiceLB (`type: LoadBalancer`) instead of a hostPort DaemonSet, so rollouts no longer hang. One replica in the base, `maxUnavailable: 0, maxSurge: 1`, a PodDisruptionBudget with `maxUnavailable: 1`, `platform-critical` priority and `CreateReplace` CRD handling. The hcloud CCM Load Balancer controller is off so ServiceLB stays the only answer.

- Fix node drains on a single-instance CNPG cluster (`enablePDB: false`). The restore-test Role can patch and delete only `postgres-cluster-restore-test`, and the restore cluster takes its StorageClass from `RESTORE_STORAGE_CLASS`. The hcloud base adds the `hcloud-volumes-delete` StorageClass for it.

- Breaking: Prometheus reads rules, ServiceMonitors and PodMonitors only from namespaces labelled `app.kubernetes.io/part-of: baukit-platform`, which loads the restore-test alert in `postgres`. Application namespaces need the label to be scraped. Grafana loads dashboards from every namespace into folders named by the `grafana_folder` annotation.

- Breaking: the hcloud base no longer manages the `kube-system` Namespace. Stateful Namespaces, the CNPG Cluster and the local RustFS PVC carry `kustomize.toolkit.fluxcd.io/prune: disabled`.

- Harden the local lifecycle: the snapshot holds only files Git would commit, the snapshot server listens on the k3d network gateway, runs as non-root and validates `Content-Length`, the k3d API and ports 80/443 bind `127.0.0.1`, and `status` prints where secrets live instead of their values.

- Breaking: remove the Keycloak operator base (`keycloak/`, vendored CRDs, realm import and migration Job examples) and the local cluster's Keycloak and realm components (BK21). Production signs in with Clerk or WorkOS; the local fixture verifies tokens from its compose Keycloak on the host.

- Group worker metrics by `job_kind` in shared dashboards, recording rules, and alerts. Test distinct handler types under one scrape job.

- Replace MinIO with digest-pinned RustFS 1.0.1 in the local platform. Use AWS CLI bucket bootstrap, keep `baukit-local` and `posthog`, and connect local Loki, PostHog, and PostgreSQL backups through path-style S3 in `us-east-1`.

- Refresh platform charts within their current majors and vendor Barman Cloud 0.15.1. Update PostHog Redis to 6.2.24, ZooKeeper to 3.9.5, Redpanda to 25.3.17, and BusyBox to 1.38. PostgreSQL stays on 18.6 and PostHog PostgreSQL on 14.1.

- Pin the CNPG operator default, primary and restore-test clusters to PostgreSQL
  `18.6-system-trixie`.
