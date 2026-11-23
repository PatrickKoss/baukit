# BaukitRedisAclLoadFailing

## Meaning

The `acl-reload` container of a redis-ha pod (composed with `components/redis-ha-acl`) ran `ACL LOAD`
after Secret `platform-redis/redis-ha-acl` changed, and Redis rejected the new `users.acl` for
10 minutes. Redis keeps the users it loaded last, and `acl-reload` retries once a minute.

## Impact

Users added or changed in the Secret cannot log in, so a tenant whose user is new fails to connect to
its rate-limit store. Existing users keep working. A Redis container that restarts now reads the broken
file and does not start, so a failover or node drain takes the pod out until the file is fixed.

## First checks

```bash
kubectl -n platform-redis logs <pod> -c acl-reload --since=30m | grep 'ACL LOAD'
kubectl -n platform-redis get secret redis-ha-acl -o jsonpath='{.metadata.annotations}'
```

The log names the line of `users.acl` Redis rejected and the reason. It replaces quoted tokens with
`<redacted>`, because the token can be a password.

```promql
increase(redis_commands_failed_calls_total{cmd="acl|load"}[5m])
```

## Fix

- Fix the line in the Secret's source (for platform-infra, the SOPS file behind
  `local-secrets-redis`) and let Flux apply it. `acl-reload` loads the fixed file within a few
  seconds of the kubelet updating the volume, about a minute after the Secret changes.
- `Duplicate user`: every user name appears once. `baukit-acl-reload` is reserved for the reload
  user that `acl-reload` appends.
- `Unknown command or category name`: a `+command` the Redis version does not know. Check the
  spelling against `COMMAND LIST`.

## Escalation

Page the platform owner if a redis-ha pod is not running while the alert fires. Sentinel needs two of
the three pods to fail over, and a pod that restarts stays down until the file is fixed.
