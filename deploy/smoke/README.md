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

The cluster API and the default port mapping bind to `127.0.0.1`. The harness writes the cluster's kubeconfig to its temporary directory and never changes the default kubeconfig. `BAUKIT_SMOKE_K3D_API_PORT` (default: a free port on `127.0.0.1`) and `BAUKIT_SMOKE_K3D_PORTS` change them.

Set `BAUKIT_SMOKE_PRODUCT_CHECK` to an executable product-owned script for CRUD, worker, or persistence assertions. It receives `BAUKIT_SMOKE_API_URL` and `BAUKIT_SMOKE_ACCESS_TOKEN_FILE`; the token file is mode 0600 and is deleted with the disposable working directory. The token is automatically added to the log leak scan. This extension point keeps product payloads out of the shared skeleton.

Example invocation shape:

```sh
BAUKIT_SMOKE_PRODUCT=my-product \
BAUKIT_SMOKE_VALUES_FILE="$PWD/deploy/environments/local.yaml" \
BAUKIT_SMOKE_IMAGES='my-product:smoke postgres:18.6-alpine' \
BAUKIT_SMOKE_DEPENDENCIES_FILE="$PWD/deploy/local/dependencies.yaml" \
BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS=postgres \
BAUKIT_SMOKE_WORKER_ENABLED=true \
BAUKIT_SMOKE_TOKEN_COMMAND="$PWD/scripts/smoke-token.sh" \
deploy/smoke/k3d-smoke.sh
```

The token command owns sign-in. A product whose deployed issuer is Clerk or WorkOS mints a test token through that provider; a product testing against its local Keycloak realm can call its generated `scripts/pkce-login.py`. The harness never prints the token. Baukit ships no production Keycloak, so the harness no longer logs in through a Keycloak form itself.
