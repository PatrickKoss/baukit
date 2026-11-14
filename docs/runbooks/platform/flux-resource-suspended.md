# BaukitFluxResourceSuspended

## Meaning

A Flux object has had `spec.suspend: true` for 24 hours. Someone ran `flux suspend` and did not run
`flux resume`.

## Impact

Flux ignores Git for that object. Fixes and security updates merged since then never reach the
cluster, and hand edits in the cluster stay in place without drift correction.

## First checks

```bash
flux get all -A | grep -i 'suspended.*true'
kubectl -n <namespace> get <kind> <name> -o jsonpath='{.spec.suspend}{"\n"}'
kubectl -n <namespace> get events --field-selector involvedObject.name=<name>
```

```promql
gotk_resource_info{suspended="true"}
```

Ask in the platform channel who suspended it and why. Flux does not record the actor; the Kubernetes
audit log shows the `patch` on the object.

## Fix

- The reason is gone: `flux resume <kind> <name> -n <namespace>` and watch it turn Ready.
- The suspension is still needed: record the reason and an end date in the platform channel, and remove
  the object from Git instead if it should stay off for good.
- A suspended object committed to Git with `suspend: true` is a mistake; remove the field in Git.

## Escalation

Escalate to the platform owner when nobody owns the suspension after one working day.
