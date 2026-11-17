# BaukitPostgresRestoreTestStale

## Meaning

`baukit_restore_test_last_success_timestamp_seconds{cluster="postgres-cluster"}` is older than 35
days for one hour, or absent for one hour once the `postgres-restore-test` CronJob
(`kube_cronjob_created`) is older than 35 days. A cluster younger than that has had no scheduled run
yet and stays quiet. The monthly `postgres-restore-test` CronJob in namespace `postgres`
has not proved that the latest backup restores. The metric lives only in the
`restore-test-pushgateway` pod's memory, so a restarted Pushgateway also makes it absent.

## Impact

Nothing is broken for users today. Nobody knows whether the backups can be restored, so a data-loss
incident could turn out unrecoverable.

## First checks

Age of the last proof in days:

```promql
(time() - baukit_restore_test_last_success_timestamp_seconds{cluster="postgres-cluster"}) / 86400
```

The CronJob is suspended in the base until an overlay sets `spec.suspend: false`:

```sh
kubectl -n postgres get cronjob postgres-restore-test -o jsonpath='{.spec.suspend} {.status.lastSuccessfulTime}{"\n"}'
kubectl -n postgres get jobs --sort-by=.metadata.creationTimestamp
kubectl -n postgres logs job/<latest-job> --tail=100
```

Did the Pushgateway restart and lose the value?

```sh
kubectl -n postgres get pods -l app.kubernetes.io/name=restore-test-pushgateway
```

Backups and the object store the restore reads from:

```sh
kubectl -n postgres get backups.postgresql.cnpg.io --sort-by=.metadata.creationTimestamp | tail -5
kubectl -n postgres get objectstores.barmancloud.cnpg.io postgres-backups -o yaml
```

## Fix

- Suspended: set `spec.suspend: false` in the overlay, or run it now with
  `kubectl -n postgres create job --from=cronjob/postgres-restore-test restore-test-manual`.
- Pushgateway restarted: run the job by hand as above. The proof itself was fine.
- The restored cluster never reaches `Ready`: read the `postgres-cluster-restore-test` pod logs
  before the job's cleanup deletes it, and fix the object store credentials or the missing WAL.

## Escalation

Platform on-call during working hours. If the restore itself fails, treat it as critical and tell
every product owner whose database lives in `postgres-cluster`.
