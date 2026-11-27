# Baukit k3d smoke harness

`k3d-smoke.sh` is a chart-paired skeleton for a disposable, production-shaped local deployment. It creates a new k3d cluster, imports already-built application and dependency images, applies product-owned dependencies and Secrets, installs `baukit-app`, installs the observability provisioning chart, calls the API with an access token from a product-owned token command, verifies metrics and log redaction, and deletes the cluster on exit.

The harness deliberately contains no build steps, migrations, realm data, CRUD payloads, database assertions, or other product logic. Put those in the product repository. Migration execution remains the application chart's pre-install hook.

Required settings:

| Setting | Purpose |
|---|---|
| `BAUKIT_SMOKE_PRODUCT` | Stable product name and default namespace/release prefix. |
| `BAUKIT_SMOKE_VALUES_FILE` | Product values file for `baukit-app`. |
| `BAUKIT_SMOKE_IMAGES` | Whitespace-separated local images imported into k3d, including dependency images. |
| `BAUKIT_SMOKE_TOKEN_COMMAND` | Executable that writes one access token to the file named by `BAUKIT_SMOKE_ACCESS_TOKEN_FILE`. |

Common optional settings include `BAUKIT_SMOKE_DEPENDENCIES_FILE`, whitespace-separated `BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS`, colon-separated `BAUKIT_SMOKE_SECRET_MANIFESTS`, `BAUKIT_SMOKE_WORKER_ENABLED=true`, `BAUKIT_SMOKE_KNOWN_GAPS_FILE`, and `BAUKIT_SMOKE_RUNBOOK_BASE_URL` (the observability chart's required `runbookBaseUrl`; it defaults to Baukit's `docs/runbooks/app` on GitHub). `BAUKIT_SMOKE_RESOURCE_NAME` must match `fullnameOverride` when that differs from the Helm release. The harness enables `networkPolicy.dns.k3sCompatible` by default; set `BAUKIT_SMOKE_K3S_DNS_ENABLED=false` only when the selected CNI resolves the pod-selected CoreDNS rule correctly.

The harness creates the product namespace with Pod Security `enforce`, `audit` and `warn` at `restricted` (`BAUKIT_SMOKE_POD_SECURITY_LEVEL`) before it applies anything to it, so every pod there is admitted under that level. Product dependencies such as PostgreSQL run in a separate namespace, `<namespace>-deps` by default (`BAUKIT_SMOKE_DEPENDENCIES_NAMESPACE`), labelled `baseline` (`BAUKIT_SMOKE_DEPENDENCIES_POD_SECURITY_LEVEL`) because the stock postgres image runs as root. The harness adds no NetworkPolicy to the dependency namespace, so it admits the product's traffic. Point the product at `<svc>.<namespace>-deps.svc` in its Secret and allow that egress through `networkPolicy.additionalEgress` with a `namespaceSelector` on `kubernetes.io/metadata.name`. Secrets stay in the product namespace. At the end the smoke fails if the namespace lost its enforce label or its events mention a PodSecurity violation. `BAUKIT_SMOKE_POLICY_DIR` names a directory of admission policies that the harness applies with `kubectl apply --recursive` before it creates the namespaces.

When every pull-through mirror from `deploy/platform/registry-mirrors.sh up` is running, the harness creates the cluster with them (`--registry-config` and one `--registry-use` per mirror) and detaches them from the cluster network before it deletes the cluster. If any mirror is missing or stopped, it creates the cluster without mirrors and pulls from the upstream registries.

## Kyverno in Enforce mode

Before it creates any namespace, the harness runs `kyverno-enforce.sh install`. It applies `deploy/platform/cluster-baseline/priority-classes.yaml`, because the Kyverno values set `platform-critical` and `platform-standard`. It then installs the Kyverno chart that `deploy/platform/kyverno/oci-repository.yaml` pins (OCI URL, tag and digest; the pull fails on another digest), with the values of `helm-release.yaml`. The four ServiceMonitors are switched off because the smoke cluster has no Prometheus Operator. It then creates `kyverno/baukit-registry-allowlist` with the `BAUKIT_SMOKE_IMAGES` references as the only allowed prefixes, resolved the way the policy resolves Docker Hub short names (`postgres:18` becomes `docker.io/library/postgres:18`), renders `deploy/platform/kyverno-policies` (policies and `exceptions/`) with every `ValidatingPolicy` and `ImageValidatingPolicy` switched to `validationActions: [Deny]`, applies it and waits until each policy reports `WebhookConfigured`. The overall `ready` flag is not used: it also needs reporting RBAC for kinds the smoke cluster lacks, such as Flux `Alert`. An unknown policy kind fails the render instead of staying in Audit. The upstream Pod Security chart (`pod-security-release.yaml`) is left out; Pod Security Admission at `restricted` enforces the same standard.

The product namespace carries `baukit.dev/tenant=true`, so the tenant policies apply to the chart and `generate-tenant-defaults` adds its default-deny NetworkPolicy and LimitRange. The observability namespace is created `restricted`. Policies without a tenant selector also apply to the dependency namespace, so dependency manifests need cpu and memory requests and limits and pinned image tags. Kyverno also checks the Deployments and Jobs a chart renders, so a violation fails `helm upgrade` at once with `Policy <name> failed: ...`. When the install fails, the harness also prints the namespace's warning events. Kyverno images (`reg.kyverno.io`) must be pullable from the smoke host. `kyverno-enforce.sh render <dir>` writes the chart pin, values and enforced policies without a cluster.


The cluster API and the default port mapping bind to `127.0.0.1`. The harness writes the cluster's kubeconfig to its temporary directory and never changes the default kubeconfig. `BAUKIT_SMOKE_K3D_API_PORT` (default: a free port on `127.0.0.1`) and `BAUKIT_SMOKE_K3D_PORTS` change them. The cluster runs the k3s image that `platform_k3s_image` in `deploy/platform/local-git-http.sh` pins by digest, the same one `platform-up.sh` uses, so k3d's own default k3s version never applies. The harness stops before k3d when `docker info` reports a cgroup version other than 2, because Kubernetes 1.35 and later do not support cgroup v1. `BAUKIT_SMOKE_K3S_IMAGE` overrides the image. The server also gets the absolute kubelet eviction thresholds of `platform_local_kubelet_args`, so a smoke cluster on a busy host does not taint itself with disk pressure at k3s's stock 5% free.

Set `BAUKIT_SMOKE_PRODUCT_CHECK` to an executable product-owned script for CRUD, worker, or persistence assertions. It receives `BAUKIT_SMOKE_API_URL` and `BAUKIT_SMOKE_ACCESS_TOKEN_FILE`; the token file is mode 0600 and is deleted with the disposable working directory. The token is automatically added to the log leak scan. This extension point keeps product payloads out of the shared skeleton.

Example invocation shape:

```sh
BAUKIT_SMOKE_PRODUCT=my-product \
BAUKIT_SMOKE_VALUES_FILE="$PWD/deploy/environments/local.yaml" \
BAUKIT_SMOKE_IMAGES='my-product:smoke postgres:18.7-alpine@sha256:b6972e1115a4ff8eba5d7f7d19184926d1e16d7d1136ae1e1a7ebf286da0f015' \
BAUKIT_SMOKE_DEPENDENCIES_FILE="$PWD/deploy/local/dependencies.yaml" \
BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS=postgres \
BAUKIT_SMOKE_WORKER_ENABLED=true \
BAUKIT_SMOKE_TOKEN_COMMAND="$PWD/scripts/smoke-token.sh" \
deploy/smoke/k3d-smoke.sh
```

The token command owns sign-in. A product whose deployed issuer is Clerk or WorkOS mints a test token through that provider; a product testing against its local Keycloak realm can call its generated `scripts/pkce-login.py`. The harness never prints the token. Baukit ships no production Keycloak, so the harness no longer logs in through a Keycloak form itself.

## Load smoke

`api-load.js` pushes a generated API past its concurrency limit with k6 and checks how it sheds. It is
opt-in: CI does not run it, because the numbers depend on the machine.

The `overload` scenario ramps an open request rate on `GET /items` from 50 to `PEAK_RPS` in
`RAMP_SECONDS`, then holds it for `HOLD_SECONDS`. An open rate keeps arriving while the API queues, so
the queue fills and baukit-http sheds a request once it has waited longer than the 1 s queue timeout.
A `liveness` scenario polls `/healthz` on the ops listener once a second for the whole run and five
seconds after it. A `clock` scenario sleeps 100 ms in a loop and records in `clock_lag` how much
longer the wall clock says each sleep took.

| Threshold | Passes when |
|---|---|
| `shed_responses` | at least one 503 came back, so the run really went past the limit |
| `shed_without_retry_after` | every 503 carried a numeric `Retry-After` header |
| `unexpected_status` | every response was a 200 or a 503; a timeout, 429 or 500 fails |
| `admitted_duration` | the p99 of the 200 responses stayed under `ADMITTED_P99_MS`, 1500 ms by default; checked by `admitted_latency.py` after k6 exits |
| `process_up` | every `/healthz` poll answered 200, during the load and after it |

The 1500 ms target is the 1 s queue timeout plus 500 ms for the handler. An admitted request can wait
the full queue timeout before it gets a permit, so a lower target would fail a healthy API.

k6 times requests on the wall clock, so a clock step lands in every request in flight. Under load,
WSL 2 steps the clock forward by about 3.6 s every 35 s, which pushed the raw p99 to 4.6 s while the
API answered every request in about 1 s. k6 writes its samples to a CSV file, and `admitted_latency.py`
treats a `clock_lag` above 250 ms as a step and subtracts it from each admitted request that spans
it. It prints the corrected and the raw p99 and the steps it found. A real k6 stall also shows up as
`clock_lag`, so a run with steps on a host without clock steps points at a starved k6, not at the API.

### Running it against a fixture

Generate a backend-only fixture (no auth, so `/items` needs no token), start its database and run the
API with a low concurrency limit. The config prefix is the fixture name in upper case with
underscores, `LOAD_FIXTURE` here.

```bash
cargo build --manifest-path cli/Cargo.toml --bin baukit
cli/target/debug/baukit new load-fixture --backend --dir .generated-fixture --baukit-path rust
cd .generated-fixture/load-fixture
make db-up migrate
LOAD_FIXTURE__HTTP__CONCURRENCY_LIMIT=2 make run
```

Then, from the repository root in a second shell:

```bash
make load-smoke
```

`make load-smoke` calls `run-load-smoke.sh`, which runs the k6 image pinned by version and digest in
`deploy/tools.toml` with `docker run --network host`, so k6 reaches the API on `127.0.0.1`. Host
networking reaches the host's loopback only on Linux. The runner exits 99 when a threshold fails.

The script passes these variables into the container only when they are set:

| Variable | Default |
|---|---|
| `API_URL` | `http://127.0.0.1:8080` |
| `OPS_URL` | `http://127.0.0.1:9090` |
| `SMOKE_PATH` | `/items` |
| `SMOKE_TOKEN` | empty; sent as a bearer token when set, for fixtures with auth |
| `PEAK_RPS` | `2000` |
| `RAMP_SECONDS` | `10` |
| `HOLD_SECONDS` | `20` |
| `MAX_VUS` | `400` |
| `ADMITTED_P99_MS` | `1500` |

If `shed_responses` fails, the API kept up with `PEAK_RPS`. Lower the concurrency limit or raise
`PEAK_RPS`. If the fixture runs the rate limiter, its 429s count as unexpected. Turn the limiter off
for the run, since the smoke measures the concurrency gate.
