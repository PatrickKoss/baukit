# BaukitWslStartTimeDriftInhibitor

## Meaning

Only local clusters on WSL2 load this rule (`components.wsl` in the platform pack). The WSL2 kernel's
wall clock runs ahead of `CLOCK_BOOTTIME`, about 200 s per hour on the reference host. The kernel
computes `/proc/stat` btime as wall time minus boot time on every read, so btime grows in 1 s
steps. Go exporters report `process_start_time_seconds` as btime plus `/proc/<pid>/stat` field 22
divided by `CLK_TCK`. Field 22 stays fixed, but the sum follows btime and changes about 16 times in
10 minutes. The upstream `AlertmanagerClusterCrashlooping` counts those changes.

This alert fires when the Alertmanager start time moved, btime moved, no container restarted and
the start time moved less than five CrashLoopBackOff starts allow (150 s). Alertmanager uses it only
to inhibit `AlertmanagerClusterCrashlooping` for the same namespace and service. Nobody receives it.

## Impact

None. It hides a false crashloop alert. A real restart increases
`kube_pod_container_status_restarts_total`, which silences this alert and lets the crashloop
alert through.

## First checks

```bash
kubectl -n observability exec alertmanager-kube-prometheus-stack-alertmanager-0 -c alertmanager -- \
  sh -c 'cut -d" " -f22 /proc/1/stat; grep btime /proc/stat; cat /proc/uptime'
```

```promql
delta(node_boot_time_seconds[1h])
increase(kube_pod_container_status_restarts_total{container="alertmanager"}[10m])
```

## Fix

Nothing in the cluster. If field 22 changes or the restart counter grows, the pod really restarts:
read `kubectl -n observability describe pod` and the previous container logs.

## Escalation

None. If it fires on a host that is not WSL2, set `components.wsl: false` and investigate the clock.
