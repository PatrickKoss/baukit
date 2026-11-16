# BaukitFalcoWarningEventsHigh

## Meaning

One Falco rule matched more than 10 times in 15 minutes at priority Warning or Error, and kept doing so
for 15 minutes.

## Impact

Either something misbehaves repeatedly, or the rule is noisy. Noise trains people to ignore Falco, and it
fills Loki.

## First checks

```promql
topk(10, sum by (rule_name) (increase(falcosecurity_falco_rules_matches_total{priority=~"[34]"}[1h])))
topk(10, sum by (rule, k8s_ns_name, k8s_pod_name) (increase(falcosecurity_falcosidekick_falco_events_total{priority_raw=~"warning|error"}[1h])))
```

```logql
{source="syscall", priority=~"Warning|Error"} | json | rule = `<rule name>`
```

Look at `output_fields` in the events: `proc.cmdline`, `proc.pname`, `container.image.repository` and
`k8s.ns.name` usually show whether the match is one workload doing something on a loop.

## Fix

- A real problem (a crash-looping job that writes below `/etc`, a container that probes the service
  account directory): fix the workload.
- Events without `k8s.ns.name` come from processes outside the cluster that share the kernel. They
  should be dropped by `baukit_foreign_host_event`; if they are not, check that the rule appends
  `and not baukit_known_noise` in `baukit.rules.yaml`.
- A legitimate workload: add a narrow exception macro with a comment naming the source, run
  `deploy/platform/falco/validate-rules.sh`, and commit it. `deploy/platform/falco/README.md` has the
  noise-tuning steps and an example macro.

## Escalation

If the matches come from a tenant workload and the cause is not obvious within a day, open an issue
with the product owner. If they come from a platform namespace, tell the platform owner.
