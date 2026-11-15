# Platform e2e

`run.sh` (or `make platform-e2e`) builds a disposable k3d cluster, composes the platform with Flux from a
snapshot of the working tree and runs the chainsaw suites in `tests/`. It deletes the cluster on exit,
also on Ctrl-C.

```bash
deploy/platform/e2e/run.sh                     # every suite
deploy/platform/e2e/run.sh --suite smoke       # one suite
deploy/platform/e2e/run.sh --keep              # keep the cluster; the KUBECONFIG path is printed
deploy/platform/e2e/run.sh --cluster-name ph-x # another cluster name
deploy/platform/e2e/run.sh --without kyverno-policies # leave out Flux Kustomization e2e-kyverno-policies
```

The cluster comes from `k3d_create_local_cluster` in `../local-git-http.sh`, the helper the lifecycle uses. The API
and load balancer bind 127.0.0.1 and the snapshot server binds the k3d network gateway. The cluster network is the
first free `/24` in `10.89.100-199` (override with `BAUKIT_E2E_SUBNET`), inside the `nodeCIDR` of
`../overlays/local/cluster-network`, so the node-cidr NetworkPolicies admit the API server. Every base with
such a policy is composed through a component that adds `node-cidr`. Tools come from `.tools/bin` first
(`scripts/install-deploy-tools.sh`); a missing tool exits 2. The kubeconfig and snapshot live in a
private directory under `BAUKIT_E2E_STATE_DIR` (default `~/.local/state/baukit-platform-e2e`); the
default kubeconfig is never touched.

Before it creates the cluster the runner takes `flock` on
`${XDG_CACHE_HOME:-~/.cache}/baukit/platform-e2e.lock` (override with `BAUKIT_E2E_CLUSTER_LOCK`).
When `BAUKIT_HEAVY_SLOT_DIR` is set, it then waits, polling every 30 seconds, while both `slot-1` and
`slot-2` exist in that directory. Without it there is no slot wait. After k3d creates the cluster it
waits up to 2 minutes for a node to register and prints the server's last 30 log lines if none does.
The cluster runs on k3s's embedded etcd (`--cluster-init`). The default SQLite datastore through kine
exits fatally on a failed commit when a starved host cancels its request, which restarted servers
mid-run. It holds no product
gate lock such as `android.lock` or `browser.lock`.

`--without NAME` drops `e2e-NAME` from the rendered overlay and from every `dependsOn` list. It is for
debugging and for measuring while a known blocker is open; a run with it is not a passing gate. If the
platform does not converge, the runner prints every Kustomization and HelmRelease that is not Ready
with its condition message.

`overlay/` holds the GitRepository and one Flux Kustomization per base. It composes every base except
`hcloud` and `posthog`, plus:

- `components/observability`: the local observability component, which brings the `alertmanager-routing`
  Secret and `alert-sink` in `observability`. The sink stores each notification and serves them on
  `GET /alerts`;
- `components/kyverno-policies`: the policy library plus the registry allowlist, which admits the
  public registries the platform pulls from and `localhost/baukit-e2e/`;
- `components/tenant`: the `e2e-tenant` namespace built from `../components/tenant` and the echo
  app, deployed with `deploy/chart/baukit-app` from the image the runner builds in `echo/`.

`redis` is left out because it and `redis-ha` own the same objects in `platform-redis`; the overlay
composes `redis-ha`.

The last line, `platform-e2e cluster time Ns, exit S`, counts from cluster creation to the end of cleanup and leaves out lock and slot waits.

Artifacts land in `.ci-gate/platform-e2e/`: the chainsaw JUnit report, events, policy reports, Flux
status, NetworkPolicies, Jobs, pod logs and `docker-stats.log`, sampled every 15 seconds.

## Adding a suite

Add `tests/<feature>/chainsaw-test.yaml`. Copy `smoke/` for cluster-wide checks and `alert-pipeline/` for
a test that creates objects and polls for an effect.
