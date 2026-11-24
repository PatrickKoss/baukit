# Changelog

## [Unreleased]

## [0.30.2] - 2026-11-24

## [0.30.1] - 2026-11-24

## [0.30.0] - 2026-11-23

## [0.29.0] - 2026-11-23

- A worker can opt in to the backing Redis with the new `worker.redis.enabled`, default `false`. With it on, `backing.redis.mode` set and the worker enabled, the worker receives `<PREFIX>__RATE_LIMIT__REDIS_KEY_NAMESPACE`, in `dedicated` mode `<PREFIX>__RATE_LIMIT__REDIS_URL`, a `<redis>-worker-egress` NetworkPolicy and admission in `<redis>-ingress`, and in `shared` mode the `baukit.dev/redis-client` label and the `platform-redis` rule in its backing egress. `config.existingSecretRefs` already reached the worker. With the default, a release renders as before. A product that turns it on and also set the worker's Redis URL in `worker.env` must drop that entry, or the container lists the variable twice and server-side apply rejects it.

## [0.28.8] - 2026-11-22

## [0.28.7] - 2026-11-22

- New `backing.postgres.mode` and `backing.redis.mode` (`shared` or `dedicated`, empty by default). `shared` adds the platform client label and egress; `dedicated` Postgres reads `<PREFIX>__DATABASE__URL` from the CNPG Secret `<cluster>-app` and allows egress to that cluster's pods; `dedicated` Redis turns on the chart's Redis and sets `<PREFIX>__RATE_LIMIT__REDIS_URL`. With either Redis mode the API gets `<PREFIX>__RATE_LIMIT__REDIS_KEY_NAMESPACE`, `<product>-<deploymentEnvironment>:` unless `backing.redis.keyNamespace` or an override sets it. The backing egress goes into a separate `<release>-<process>-backing-egress` NetworkPolicy. With empty modes the output is unchanged.

## [0.28.6] - 2026-11-22

## [0.28.5] - 2026-11-22

## [0.28.4] - 2026-11-19

- `baukit-app.image` renders `repository:tag@digest` when an image sets both `tag` and `digest`. It rendered `repository@digest` and dropped the tag, unlike the platform's own pins. The runtime still pulls by the digest. Without a tag the reference stays `repository@digest`, and without a digest `repository:tag`.

## [0.28.3] - 2026-11-19

## [0.28.2] - 2026-11-18

## [0.28.1] - 2026-11-18

- The default Redis image is `8.10.3-alpine`, and `redis.image.digest` pins it by digest, so the rendered image is `redis@sha256:7958cfbb9d152f87dadfbc89f1b0b15938f0b81ff8982feb327476f4911935c3`.

## [0.28.0] - 2026-11-17

## [0.27.4] - 2026-11-17

## [0.27.3] - 2026-11-17

## [0.27.2] - 2026-11-17

## [0.27.1] - 2026-11-17

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
