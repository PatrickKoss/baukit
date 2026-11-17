# BaukitBuildInfoAbsent

## Meaning

A target is up but does not expose `build_info{version, commit, rust_version}` for 10 minutes. The
telemetry spec requires every process to emit it.

## Impact

Dashboards cannot tell which version runs, and release correlation breaks. Traffic is not affected.

## First checks

Targets without the series:

```promql
(up{product!="", service!=""} == 1) unless on (product, service, environment, namespace, instance, job) (build_info{product!="", service!=""} == 1)
```

What the pod actually exports:

```sh
kubectl -n <namespace> port-forward pod/<pod> 9090:9090 &
curl -s http://127.0.0.1:9090/metrics | grep -E '^build_info'
```

Version of the image that runs:

```sh
kubectl -n <namespace> get pod <pod> -o jsonpath='{.spec.containers[0].image}{"\n"}'
```

## Fix

- The binary does not register build info: pass `build_info!()` from `baukit-runtime` into
  `ServiceInfo::new`, as the generated `src/bin/api.rs` does, and redeploy.
- A non-baukit process carries a `service` label by mistake: drop that relabeling from its
  ServiceMonitor.

## Escalation

Ticket for the product team. No paging.
