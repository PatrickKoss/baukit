# BaukitFalcoEventsDropped

## Meaning

`falcosecurity_scap_n_drops_total` rose on a Falco pod for 10 minutes. The kernel ring buffer filled
up before Falco read it, so the kernel threw syscall events away.

## Impact

Rules never see the dropped events. An attacker who floods syscalls can hide activity in the gap.

## First checks

```promql
sum by (pod, drop) (rate(falcosecurity_scap_n_drops_buffer_total[5m]))
sum by (pod) (rate(falcosecurity_scap_n_evts_total[5m]))
falcosecurity_falco_cpu_usage_ratio
```

```bash
kubectl -n falco top pod -l app.kubernetes.io/name=falco
kubectl -n falco logs -l app.kubernetes.io/name=falco -c falco --tail=200 | grep -i drop
```

The `drop` label of `falcosecurity_scap_n_drops_buffer_total` names the syscall family that overflowed
(`open`, `execve`, `connect`, `clone_fork`, `proc`, `close`, `dir_file`, `other_interest`).

## Fix

- Falco is CPU throttled: `falcosecurity_falco_cpu_usage_ratio` sits near the 1000m limit. Raise the
  CPU limit in the overlay.
- A workload produces a syscall storm (a build, a crash loop, a backup): find it with
  `topk(5, sum by (k8s_ns_name, k8s_pod_name) (rate(falcosecurity_falcosidekick_falco_events_total[5m])))`
  and the node's `top`. Fix the workload rather than the buffer.
- Short bursts during node boot or image pulls are expected. If drops recur under normal load, raise
  `driver.modernEbpf.bufSizePreset` in the HelmRelease and measure memory again.

## Escalation

If drops continue for more than an hour and none of the above explains them, tell the platform owner:
detection coverage on that node is partial.
