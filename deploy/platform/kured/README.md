# kured

This optional base installs kured chart `6.1.0` (kured `1.23.0`) in the `kured`
namespace as `platform-standard`. It checks hourly and permits a coordinated
reboot on Sunday between 03:00 and 05:00 `Etc/UTC`. Drains time out after 30
minutes and forced reboot is disabled. A ClusterIP metrics Service and
ServiceMonitor are enabled.

## Alert check

Before a reboot kured asks the in-cluster Prometheus
(`kube-prometheus-stack-prometheus.observability:9090`) for alerts and skips
the reboot while any of them fires. `alertFiringOnly` ignores pending alerts.
kured filters by alert name only, so `alertFilterRegexp` lists every alert with
severity `info` or `none` (including the always-firing `Watchdog`). The
`kured-alert-filter` convention rule fails when a quiet alert is missing from
the regex or a `warning`/`critical` alert matches it, so a new upstream info
alert shows up in `validate.sh`. Alerts a product ships in its own namespace
count too; keep their info alerts out of Prometheus or extend the regex in the
overlay.

`network-policy.yaml` lets kured reach Prometheus on 9090, and the observability
base admits it.

## Notifications

`notifyUrl` is empty in the base. The overlay sets a
[shoutrrr](https://containrrr.dev/shoutrrr/services/overview/) URL. The local
overlay (`overlays/local/components/kured`) posts drain and reboot messages to
the local alert sink:
`generic://alert-sink.observability.svc.cluster.local:8080/alerts?template=json&disabletls=yes`,
with a NetworkPolicy on each side. Receiver URLs that carry credentials belong
in an overlay-managed Secret referenced through `extraEnvVars`
(`KURED_NOTIFY_URL`), never in this base.

The ServiceMonitor requires the Prometheus Operator CRDs from the observability
stack before Helm applies this release. A consuming Flux overlay should express
that ordering. The overlay may patch the maintenance days, window and time
zone.

kured is privileged and mounts the host reboot sentinel by design. Include the
base only on nodes where that operational authority is intended.
