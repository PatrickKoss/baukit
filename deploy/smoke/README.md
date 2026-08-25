# Baukit k3d smoke harness

`k3d-smoke.sh` is a chart-paired skeleton for a disposable, production-shaped local deployment. It creates a new k3d cluster, imports already-built application and dependency images, applies product-owned dependencies and Secrets, installs `baukit-app`, installs the observability provisioning chart, exercises a headless OIDC authorization-code + PKCE login, verifies metrics and log redaction, and deletes the cluster on exit.

The harness deliberately contains no build steps, migrations, realm data, CRUD payloads, database assertions, or other product logic. Put those in the product repository. Migration execution remains the application chart's pre-install hook.

Required settings:

| Setting | Purpose |
|---|---|
| `BAUKIT_SMOKE_PRODUCT` | Stable product name and default namespace/release prefix. |
| `BAUKIT_SMOKE_VALUES_FILE` | Product values file for `baukit-app`. |
| `BAUKIT_SMOKE_IMAGES` | Whitespace-separated local images imported into k3d, including dependency images. |
| `BAUKIT_SMOKE_ISSUER` | Browser-visible OIDC issuer used by discovery. |
| `BAUKIT_SMOKE_OIDC_CLIENT_ID` | Public PKCE client ID. |
| `BAUKIT_SMOKE_OIDC_USERNAME` / `BAUKIT_SMOKE_OIDC_PASSWORD` | Disposable smoke user credentials. |

Common optional settings include `BAUKIT_SMOKE_DEPENDENCIES_FILE`, whitespace-separated `BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS`, colon-separated `BAUKIT_SMOKE_SECRET_MANIFESTS`, `BAUKIT_SMOKE_WORKER_ENABLED=true`, and `BAUKIT_SMOKE_KNOWN_GAPS_FILE`. `BAUKIT_SMOKE_RESOURCE_NAME` must match `fullnameOverride` when that differs from the Helm release. The harness enables `networkPolicy.dns.k3sCompatible` by default; set `BAUKIT_SMOKE_K3S_DNS_ENABLED=false` only when the selected CNI resolves the pod-selected CoreDNS rule correctly.

Set `BAUKIT_SMOKE_PRODUCT_CHECK` to an executable product-owned script for CRUD, worker, or persistence assertions. It receives `BAUKIT_SMOKE_API_URL` and `BAUKIT_SMOKE_ACCESS_TOKEN_FILE`; the token file is mode 0600 and is deleted with the disposable working directory. Its token and the OIDC password are automatically added to the log leak scan. This extension point keeps product payloads out of the shared skeleton.

Example invocation shape:

```sh
BAUKIT_SMOKE_PRODUCT=my-product \
BAUKIT_SMOKE_VALUES_FILE="$PWD/deploy/environments/local.yaml" \
BAUKIT_SMOKE_IMAGES='my-product:smoke postgres:17-alpine quay.io/keycloak/keycloak:26.7.0' \
BAUKIT_SMOKE_DEPENDENCIES_FILE="$PWD/deploy/local/dependencies.yaml" \
BAUKIT_SMOKE_DEPENDENCY_DEPLOYMENTS='postgres keycloak' \
BAUKIT_SMOKE_WORKER_ENABLED=true \
BAUKIT_SMOKE_ISSUER=http://host.k3d.internal:8081/realms/my-product \
BAUKIT_SMOKE_OIDC_CLIENT_ID=my-product-web \
BAUKIT_SMOKE_OIDC_USERNAME=test \
BAUKIT_SMOKE_OIDC_PASSWORD=password \
deploy/smoke/k3d-smoke.sh
```

`headless-pkce-login.py` can also be used independently. It discovers standard OIDC endpoints, submits a Keycloak-compatible login form, validates `state`, exchanges the S256 authorization code, optionally stores the token in a mode-0600 file, and can probe one authenticated URL. It never prints the token.
