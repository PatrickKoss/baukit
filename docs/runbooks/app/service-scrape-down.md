# BaukitServiceScrapeDown

## Meaning

Prometheus failed to scrape one target that carries a `service` label (`up == 0`) for 2 minutes.
The target is a pod's ops listener on port `metrics` (9090), path `/metrics`.

## Impact

Metrics for that pod are missing, so every other alert on it is blind. The pod itself may also be
down. A scrape that fails because the pod is gone usually comes with user-visible errors.

## First checks

Which targets are down:

```promql
up{product!="", service!=""} == 0
```

Pod state behind the `instance` label:

```sh
kubectl -n <namespace> get pods -o wide | grep <pod-ip>
kubectl -n <namespace> describe pod <pod>
kubectl -n <namespace> logs <pod> --previous --tail=100
```

The ops listener from inside the cluster:

```sh
kubectl -n <namespace> port-forward pod/<pod> 9090:9090 &
curl -sf http://127.0.0.1:9090/healthz && curl -sf http://127.0.0.1:9090/metrics | head
```

The scrape error Prometheus reports:

```sh
kubectl -n observability port-forward svc/kube-prometheus-stack-prometheus 9090:9090 &
curl -s 'http://127.0.0.1:9090/api/v1/targets?state=active' | jq '.data.activeTargets[] | select(.health=="down") | {scrapeUrl, lastError}'
```

## Fix

- CrashLoopBackOff or OOMKilled: fix the crash from `--previous` logs, or raise the memory limit.
- Pod is fine but the scrape times out: a NetworkPolicy blocks Prometheus. The chart's policy must
  admit the `observability` namespace on port 9090.
- The ops listener is disabled in config: re-enable it, since the readiness probe also uses it.

## Escalation

Product team for crashes. Platform on-call for network policy or Prometheus problems, or when
targets of several products are down at once.
