# BaukitKuredRebootBlocked

## Meaning

A node has been cordoned for an hour and still needs a reboot. kured drained it but the reboot or the
uncordon did not happen.

## Impact

The node takes no new pods. On a single-node cluster nothing new can start.

## First checks

```bash
kubectl get node <node> -o jsonpath='{.spec.unschedulable} {.metadata.annotations}'
kubectl -n kured logs -l app.kubernetes.io/name=kured --field-selector spec.nodeName=<node> --since=2h
kubectl get pods -A --field-selector spec.nodeName=<node> | grep -v Running
```

```promql
kured_reboot_required{node="<node>"}
kube_node_spec_unschedulable{node="<node>"}
```

## Fix

- Drain is stuck on a PodDisruptionBudget: find the pod with `kubectl get pdb -A` and fix the budget
  or scale the workload.
- The reboot command failed: the kured log shows the error; reboot the node by hand.
- Someone cordoned the node by hand: uncordon it with `kubectl uncordon <node>` once they are done.

## Escalation

Page the platform owner if the node is the only node of the cluster.
