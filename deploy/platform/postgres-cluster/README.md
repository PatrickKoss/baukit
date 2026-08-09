# Reusable PostgreSQL cluster base

This base is a reusable starting point for one CloudNativePG cluster. It creates
the `postgres` namespace, one deliberately non-HA PostgreSQL instance, a 10 GiB
PVC on the cluster's default StorageClass, plugin-based WAL/base backups, a
daily base-backup schedule, and a suspended monthly restore proof. It is a
template: consumers should patch names and instantiate it once per desired
cluster rather than treating it as a platform singleton.

The cluster uses only the Barman Cloud CNPG-I plugin path: `ObjectStore`,
`Cluster.spec.plugins` with `isWALArchiver: true`, and a plugin-method
`ScheduledBackup`. The removed-in-a-future-release in-tree backup integration is
not configured.

## Overlay requirements

Before reconciliation, an overlay must:

- reconcile the `cnpg` base (and therefore cert-manager) first, normally through
  Flux `Kustomization.spec.dependsOn`;
- replace `s3://__OVERLAY_REQUIRED__/` with its complete bucket/prefix and add
  `ObjectStore.spec.configuration.endpointURL` when using a non-AWS S3 service;
- create `postgres-backup-credentials` in this namespace with
  `ACCESS_KEY_ID` and `ACCESS_SECRET_KEY` keys (and patch optional session-token
  or CA references when the provider requires them);
- create `product-db-credentials`, type `kubernetes.io/basic-auth`, with
  `username: product_owner` and a generated `password`, or patch the initial
  database, owner, and Secret reference to the first real product;
- ensure a default StorageClass exists and patch storage/resources from measured
  load if the small defaults are insufficient;
- ensure the Prometheus Operator CRDs exist before applying the included
  `ServiceMonitor` and `PrometheusRule`, and configure Prometheus selectors to
  admit resources labeled `app.kubernetes.io/part-of: baukit-platform`;
- review the restore schedule, then explicitly set the CronJob to
  `spec.suspend: false`; add a priority class in the overlay if desired.

No endpoint, bucket, credential, database password, domain, or personal value is
included. The destination string is an intentionally unusable schema marker
that must be replaced.

## One database and one owner per product

The initial `Cluster.spec.bootstrap.initdb` pair encodes the convention as the
placeholder database `product`, owner `product_owner`, and overlay-owned
`product-db-credentials` Secret. For every subsequent product, copy
`product-database.example.yaml` into the overlay and create one `DatabaseRole`
plus one `Database`. Standalone `DatabaseRole` resources are the CNPG 1.30
GitOps-oriented role API; retain policies keep deleting a Kubernetes object from
silently deleting a production role or database. Application migrations still
own schemas and tables.

For Keycloak, the overlay convention is database `keycloak`, owner
`keycloak_owner`, and Secret `keycloak-db-credentials`. The Secret can be
referenced by both the `DatabaseRole` and the Keycloak CR.

## Restore-test freshness metric

The suspended CronJob deletes any prior throwaway cluster, restores the latest
backup as `postgres-cluster-restore-test`, waits for CNPG `Ready`, pushes
`baukit_restore_test_last_success_timestamp_seconds`, and deletes the restored
cluster. A small Pushgateway retains the metric in an `emptyDir` across
container restarts; losing the pod intentionally makes the metric absent and
therefore alerts until the proof runs again. The `ServiceMonitor` exposes it to
the kube-prometheus-stack base, and `BaukitPostgresRestoreTestStale` fires when
the metric is absent or older than 35 days.

The CronJob image matches the pinned k3s/Kubernetes version. If cluster and
ObjectStore names are patched, patch the recovery manifest embedded in the
CronJob script as well.

