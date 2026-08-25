# kured

This optional base installs kured chart `6.1.0` (kured `1.23.0`) in the `kured`
namespace. It checks hourly and permits a coordinated reboot on Sunday between
03:00 and 05:00 UTC. Drains time out after 30 minutes and forced reboot is
disabled. A ClusterIP metrics Service and ServiceMonitor are enabled; no
notification receiver is configured.

The ServiceMonitor requires the Prometheus Operator CRDs from the observability
stack before Helm applies this release. A consuming Flux overlay should express
that ordering. The overlay may patch the maintenance days/window/time zone,
Prometheus alert-check URL, `priorityClassName`, and notification receiver.
Receiver URLs or credentials belong in an overlay-managed Secret/environment
reference and must not be placed in this base.

kured is privileged and mounts the host reboot sentinel by design. Include the
base only on nodes where that operational authority is intended.
