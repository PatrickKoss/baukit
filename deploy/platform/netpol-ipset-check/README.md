# NetworkPolicy ipset check

k3s enforces NetworkPolicies with its embedded kube-router. On 2026-11-24 a k3s restart under load
(k3s v1.37.2+k3s1, kube-router v2.11.1-k3s1) left one pod out of the ipsets of the policies that
admit it. Its peers got REJECTs until the pod was recreated, and kube-router logged nothing. No k3s
release fixes it yet; k3s-io/k3s#15039 describes the same silent failure and waits on kube-router.

CronJob `observability-agents/netpol-ipset-check` runs every 10 minutes. The `ipset-save` init
container (the platform's k3s image, root, host network, NET_ADMIN only) writes `ipset save` to an
`emptyDir`. `check.py` (stdlib Python, nobody, no capabilities) lists every pod and NetworkPolicy
and exits 1 when a pod that started over 10 minutes ago is missing from the `KUBE-DST-` (ingress)
or `KUBE-SRC-` (egress) set of a policy that selects it. Set names follow kube-router's
`policyDestinationPodIPSetName` and `policySourcePodIPSetName`. A failed Job fires
`KubeJobFailed` after 15 minutes. Finished Jobs are deleted after an hour, so the alert clears an
hour after the sets are right again. PolicyException `netpol-ipset-check` lifts the host-namespace,
capability and root policies for this CronJob only.

The Job reads the ipsets of whichever node it lands on. kube-router fills the target sets with pods
from every node, so one node is enough to catch a missing pod, but not a node whose own sets are
stale.

When it fires, the Job log names the pod, IP, policy and set:

```sh
kubectl -n observability-agents logs job/<failed job>
kubectl -n <namespace> delete pod <pod>   # kube-router adds the new pod IP
```
