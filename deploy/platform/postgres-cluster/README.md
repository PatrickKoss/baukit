# Reusable PostgreSQL cluster base

This base is a reusable starting point for one CloudNativePG cluster. It creates
the `postgres` namespace, one deliberately non-HA PostgreSQL instance, a 10 GiB
PVC on the cluster's default StorageClass, plugin-based WAL/base backups, a
daily base-backup schedule, and a suspended monthly restore proof. It is a
template: consumers should patch names and instantiate it once per desired
cluster rather than treating it as a platform singleton.

The primary and restore-test clusters take PostgreSQL 18 from the `postgresql`
`ImageCatalog` in `image-catalog.yaml`, which pins
`ghcr.io/cloudnative-pg/postgresql:18.7-system-trixie` by its multi-arch index
digest, matching the operator default image flavor. A patch release changes
only the catalog. CNPG manages its own data directory and PVC mounts; the
Docker Official Image mount convention does not apply here. Recreate local
clusters and PVCs when switching major versions.

18.7 is the newest PostgreSQL release with a `system-trixie` image. PostgreSQL 19
has only `19rc2` tags on `ghcr.io/cloudnative-pg/postgresql` (checked
2026-11-15), so the major stays at 18. Moving to 19 once it is GA is a separate
migration: add a `major: 19` catalog entry, then upgrade the cluster with CNPG's
offline in-place major upgrade or a new cluster imported from the old one.

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
  `PodMonitor`, `ServiceMonitor` and `PrometheusRule`, and configure Prometheus selectors to
  admit resources labeled `app.kubernetes.io/part-of: baukit-platform`;
- review the restore schedule, then explicitly set the CronJob to
  `spec.suspend: false`;
- on a cluster whose default StorageClass retains volumes (Hetzner
  `hcloud-volumes`), set the CronJob's `RESTORE_STORAGE_CLASS` env to a
  Delete-policy class such as `hcloud-volumes-delete`.

## Node drains and PodDisruptionBudgets

The instance runs as `platform-critical`. The pushgateway, the restore-test
Job and the restore cluster it creates run as `platform-standard`.

The base runs one instance with `enablePDB: false`. CNPG's default primary
PodDisruptionBudget allows zero disruptions, so with one instance it blocks
every `kubectl drain`, and the weekly kured reboot times out. Without the PDB,
a drain evicts the only instance and the database is down until the node is
back. That is the honest trade-off of a single instance.

An overlay that runs two or more instances must set `enablePDB: true` in the
same patch (the `pdb-multi-replica` convention rule fails either mismatch), so the drain waits for a switchover instead of killing the primary.
It should also set `spec.nodeMaintenanceWindow`
(`inProgress: false, reusePVC: true`) as the default, and flip `inProgress` to
`true` only for planned maintenance on single-node storage, where CNPG then
waits for the node to return and reuses the PVC.

## Prune safety

The `postgres` Namespace and the `Cluster` carry
`kustomize.toolkit.fluxcd.io/prune: disabled`. Removing this base from a Flux
Kustomization leaves the database and its volumes in place; deleting them is a
manual step.

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

## Restore proof

`ScheduledBackup` sets `immediate: true`, so a fresh cluster gets its first base
backup when the schedule object is created instead of at the next 02:00.

The suspended CronJob proves that a backup restores with data in it, not only
that a restored cluster turns `Ready`:

1. The `write-sentinel` init container connects to `postgres-cluster-rw` as the
   product owner from `product-db-credentials` (never the superuser) and upserts
   one row into `baukit_restore_sentinel` in the `product` database. The token is
   the UTC time plus the pod name. It first retries `select 1` for up to
   `WAIT_SECONDS` (300), because a new pod's traffic is dropped until kube-router
   programs its IP and CNPG may not have created the role or database yet.
2. The main container creates a plugin `Backup` of `postgres-cluster` and waits
   up to `BACKUP_TIMEOUT_SECONDS` for phase `completed`. Phase `failed` or a
   timeout fails the job.
3. It deletes any prior throwaway cluster, restores the latest backup as
   `postgres-cluster-restore-test` (one instance, no PDB) and waits for `Ready`.
4. It reads the sentinel back as the product owner through `kubectl exec` into
   the restored primary. The password goes over stdin, not the command line. A
   missing row or a different token (an old backup) fails the job.
5. Only then it pushes `baukit_restore_test_last_success_timestamp_seconds` and
   deletes the restored cluster.
6. Each Backup it creates carries `baukit.dev/restore-test: "true"` and a
   non-controller ownerReference to the Job, whose name and uid come from the
   pod labels `batch.kubernetes.io/job-name` and
   `batch.kubernetes.io/controller-uid` through the downward API. Garbage
   collection removes the Backup when the Job goes, so
   `successfulJobsHistoryLimit: 1`, `failedJobsHistoryLimit: 2` and the Job's
   `ttlSecondsAfterFinished` bound them. Deleting a CNPG Backup object does not
   delete its data in the object store; the ObjectStore `retentionPolicy`
   removes that.

A small Pushgateway retains the metric in an `emptyDir` across container
restarts; losing the pod intentionally makes the metric absent and therefore
alerts until the proof runs again. The `ServiceMonitor` exposes it to the
kube-prometheus-stack base, and `BaukitPostgresRestoreTestStale` in the platform
pack (`deploy/observability-platform/alerts/cnpg.rules.yml`) fires when the
metric is older than 35 days, or absent once the CronJob itself is older than 35 days. The platform e2e suite
`deploy/platform/e2e/tests/backup-restore` runs the same round trip on a
throwaway cluster, deletes it and restores it. `restore-test-cronjob` runs this
CronJob once, in parallel with it.

If a product patches the initial database name away from `product`, patch the
`PGDATABASE` env of both containers as well.

The CronJob's Role can create, get, list and watch CNPG clusters, create and get
backups, but patch and delete only `postgres-cluster-restore-test` and exec only
into its first pod, `postgres-cluster-restore-test-1`. It cannot change or delete the
production `postgres-cluster`. Renaming the throwaway cluster means patching
the Role's `resourceNames` too.

The CronJob image matches the pinned k3s/Kubernetes version. If cluster and
ObjectStore names are patched, patch the recovery manifest embedded in the
CronJob script as well.

## Off-node backups for the local overlay

Today the local overlay writes WAL and base backups to the in-cluster rustfs in
`postgres` (`s3://baukit-local/postgres`). Its PVC sits on k3s `local-path`
storage inside the k3d node container, the same place as the database PVC.
`k3d cluster delete` or a broken node container takes the database and every
backup with it.

To keep backups when the node goes away, run a second object store on the host,
on a host directory, and point the ObjectStore at it. The k3d network is
`k3d-<cluster>`; the default local cluster is `baukit-local`.

```bash
cluster=baukit-local
backup_dir="$HOME/.local/share/baukit-backups"
mkdir -p "$backup_dir" && sudo chown 10001:10001 "$backup_dir"
key="$(kubectl -n postgres get secret postgres-backup-credentials -o jsonpath='{.data.ACCESS_KEY_ID}' | base64 -d)"
secret="$(kubectl -n postgres get secret postgres-backup-credentials -o jsonpath='{.data.ACCESS_SECRET_KEY}' | base64 -d)"
docker run -d --name baukit-backup-store --restart unless-stopped \
  --network "k3d-$cluster" -v "$backup_dir:/data" \
  -e RUSTFS_ACCESS_KEY="$key" -e RUSTFS_SECRET_KEY="$secret" -e RUSTFS_REGION=us-east-1 \
  "$(kubectl -n postgres get deploy rustfs -o jsonpath='{.spec.template.spec.containers[0].image}')"
store_ip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"k3d-$cluster\").IPAddress}}" baukit-backup-store)"
docker run --rm --network "k3d-$cluster" -e AWS_ACCESS_KEY_ID="$key" -e AWS_SECRET_ACCESS_KEY="$secret" \
  -e AWS_DEFAULT_REGION=us-east-1 \
  "$(kubectl -n postgres get job rustfs-create-bucket -o jsonpath='{.spec.template.spec.containers[0].image}')" \
  --endpoint-url "http://$store_ip:9000" s3api create-bucket --bucket baukit-backups
```

Then patch the local postgres component: set the ObjectStore
`destinationPath` to `s3://baukit-backups/postgres` and `endpointURL` to
`http://<store_ip>:9000`, and add a NetworkPolicy that lets
`cnpg.io/podRole: instance` pods egress to `<store_ip>/32` on TCP 9000. The
container keeps its address only while it stays on the network, so give it a
fixed one with `docker network connect --ip` if the cluster is recreated often.
After the switch, check `kubectl -n postgres get backup` for a `completed`
backup and `ls "$backup_dir"` for the `postgres/postgres-cluster` prefix.

A host bind mount into the in-cluster rustfs works too: create the cluster with
`k3d cluster create ... --volume "$backup_dir:/var/lib/baukit-backups@server:0"`
and bind the `rustfs` PVC to a static `hostPath` PersistentVolume on
`/var/lib/baukit-backups`. It needs the cluster to be recreated, so the second
store above is the cheaper change.
