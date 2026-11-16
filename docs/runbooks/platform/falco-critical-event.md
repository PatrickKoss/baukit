# BaukitFalcoCriticalEvent

## Meaning

A Falco rule matched at priority Critical or above in the last 5 minutes. The `rule_name` label names the
rule. In this platform that includes a shell or package manager in a baukit app container, a migration
job reaching anything but PostgreSQL, an app process opening the service account directory, and
`kubectl exec` into `postgres`, `kyverno` or `flux-system`.

## Impact

Something ran that the platform never runs in normal operation. Treat it as a possible compromise until
you know who did it.

## First checks

Find the event and its pod in Loki:

```logql
{source="syscall", priority=~"Critical|Alert|Emergency"} | json
```

```promql
sum by (rule_name) (increase(falcosecurity_falco_rules_matches_total{priority=~"[0-2]"}[1h]))
sum by (k8s_ns_name, k8s_pod_name, rule) (increase(falcosecurity_falcosidekick_falco_events_total{priority_raw=~"critical|alert|emergency"}[1h]))
```

Then ask who touched the pod:

```bash
kubectl -n <namespace> get pod <pod> -o wide
kubectl -n <namespace> get events --field-selector involvedObject.name=<pod>
```

Check the Kubernetes audit log for `pods/exec` or `pods/attach` on that pod at the event time.

## Fix

- A person ran `kubectl exec` on purpose: confirm with them, record the reason, and ask whether a
  debug container or a job would have done the job without a shell.
- Nobody claims it: isolate the pod (`kubectl -n <namespace> label pod <pod> quarantine=true` and a
  deny-all NetworkPolicy for that label), keep it for analysis, rotate any secret the pod could read,
  and rebuild the workload from a known image digest.
- A false positive from a new, legitimate workload: add an exception macro with a reason to
  `deploy/platform/falco/rules/baukit.rules.yaml`, validate it with
  `deploy/platform/falco/validate-rules.sh`, and keep the exception as narrow as one image or one
  process.

## Escalation

Page the platform owner immediately for any unexplained Critical event. Do not delete the pod before
someone has looked at it.
