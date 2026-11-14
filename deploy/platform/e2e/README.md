# Platform e2e

`run.sh` (or `make platform-e2e`) builds a disposable k3d cluster, composes the platform with Flux from a
snapshot of the working tree and runs the chainsaw suites in `tests/`. It deletes the cluster on exit,
also on Ctrl-C.

```bash
deploy/platform/e2e/run.sh                     # every suite
deploy/platform/e2e/run.sh --suite smoke       # one suite
deploy/platform/e2e/run.sh --keep              # keep the cluster; the KUBECONFIG path is printed
deploy/platform/e2e/run.sh --cluster-name ph-x # another cluster name
```

The cluster uses the k3s image and flags from `../lib/k3s.sh`, the same file the lifecycle reads. The API
binds 127.0.0.1 and the snapshot server binds the k3d network gateway. Tools come from `.tools/bin` first
(`scripts/install-deploy-tools.sh`); a missing tool exits 2. The kubeconfig and snapshot live in a
private directory under `BAUKIT_E2E_STATE_DIR` (default `~/.local/state/baukit-platform-e2e`); the
default kubeconfig is never touched.

Before it creates the cluster the runner takes `flock` on
`~/.cache/baukit-orchestrator/platform-hardening/k3d-full.lock` (override with `BAUKIT_E2E_CLUSTER_LOCK`)
and then on `android.lock` and `browser.lock` in `BAUKIT_GATE_LOCK_DIR`
(default `~/.cache/baukit-orchestrator/quality-security`).

`overlay/` holds the GitRepository and one Flux Kustomization per base. It composes every base except
`hcloud` and `posthog`, plus:

- `components/observability`: the local observability component with Alertmanager routing every alert to
  the webhook sink;
- `components/receiver`: the sink in `e2e-receiver`. It stores each notification and serves them on
  `GET /alerts`, which tests read through the API server's service proxy;
- `components/tenant`: the `e2e-tenant` namespace (`baukit.dev/tenant=true`, PSA restricted) and the echo
  app, deployed with `deploy/chart/baukit-app` from the image the runner builds in `echo/`.

`redis` is left out because it and `redis-ha` own the same objects in `platform-redis`; the overlay
composes `redis-ha`.

Artifacts land in `.ci-gate/platform-e2e/`: the chainsaw JUnit report, events, policy reports, Flux
status, pod logs and `docker-stats.log`, sampled every 15 seconds.

## Adding a suite

Add `tests/<feature>/chainsaw-test.yaml`. Copy `smoke/` for cluster-wide checks and `alert-pipeline/` for
a test that creates objects and polls for an effect.
