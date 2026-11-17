# Changelog

## [Unreleased]

## [0.27.0] - 2026-11-17

## [0.26.7] - 2026-11-17

## [0.26.6] - 2026-11-17

## [0.26.5] - 2026-11-17

## [0.26.4] - 2026-11-16

- Breaking: the API and worker ServiceMonitors set `honorLabels: false`. Application labels that clash with target labels arrive as `exported_<name>`, so `fixed_window_amount_budget_decisions_total` carries the budget namespace in `exported_namespace`. Tenant namespaces on the baukit platform refuse `honorLabels: true`.

## [0.26.3] - 2026-11-16

## [0.26.2] - 2026-11-16

## [0.26.1] - 2026-11-16

## [0.26.0] - 2026-11-16

- Breaking: images have no default tag. Each image needs `image.tag` or `image.digest`, and `latest` fails the schema and the render. A digest renders `repository@sha256:...`.
- Breaking: `values.schema.json` rejects unknown keys and wrong types.
- Breaking: the default ingress-controller peer is the Traefik base (namespace `traefik`, `app.kubernetes.io/instance: traefik-traefik`) instead of `kube-system`.
- Breaking: PodDisruptionBudgets are on by default and render when a process runs more than one replica or the API autoscales. Without `minAvailable` or `maxUnavailable` they use `maxUnavailable: 1`.
- Breaking: the chart needs Kubernetes 1.30 for the native `preStop` sleep. The API sleeps `api.preStopSleepSeconds` (5) before SIGTERM, and `SHUTDOWN__DRAIN_TIMEOUT` is the grace period minus that sleep.
- Add a release ServiceAccount with `automountServiceAccountToken: false`, used by every pod.
- Add startup probes for the API and worker, a soft hostname topology spread for API, worker and web, `ttlSecondsAfterFinished` on the migration and seed Jobs, and an `emptyDir` `/tmp` mount for every application process.
- The render fails when `<process>.volumes` names a volume `tmp` or `<process>.volumeMounts` mounts `tmp` or `/tmp`, since the chart already provides that mount. Drop those entries from your values.

## [0.25.2] - 2026-11-13

## [0.25.1] - 2026-11-13

## [0.25.0] - 2026-11-13

## [0.24.0] - 2026-11-12

## [0.23.0] - 2026-11-12

## [0.22.0] - 2026-11-11

## [0.21.0] - 2026-11-10

## [0.20.0] - 2026-11-10

## [0.19.2] - 2026-11-08

## [0.19.1] - 2026-11-08

## [0.19.0] - 2026-11-07

## [0.18.3] - 2026-11-07

## [0.18.2] - 2026-11-06

## [0.18.1] - 2026-11-05

## [0.18.0] - 2026-11-05

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

## [0.15.0] - 2026-11-02

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

- Add optional API and worker PodDisruptionBudgets through `api.podDisruptionBudget` and `worker.podDisruptionBudget`, off by default. Each takes exactly one of `minAvailable` or `maxUnavailable`.

## [0.13.0] - 2026-10-31

## [0.12.2] - 2026-10-31

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Add optional API-only existing Secret references for current and previous platform receiver secrets.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Map `mcp.jwksUri` to the API MCP signing-key endpoint. Keep the public issuer separate.
- Cut chart changelog release sections during release preparation.
