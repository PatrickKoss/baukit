# CloudNativePG operator base

This Flux-native base creates `cnpg-system`, installs the CloudNativePG operator
with the `cloudnative-pg` chart pinned to `0.29.2` (operator `1.30.2`), and
installs the Barman Cloud CNPG-I plugin pinned to `v0.16.0`. Both controllers
have small, bounded resources. The plugin must run in the operator namespace and
uses cert-manager for its client and server certificates.

The operator's `POSTGRES_IMAGE_NAME` pins new clusters to
`ghcr.io/cloudnative-pg/postgresql:18.7-system-trixie` by digest. The reusable
primary and restore-test manifests reference the same image through the
`postgresql` `ImageCatalog` in the `postgres-cluster` base.

The base is intentionally cluster-wide so reusable `postgres-cluster` instances
can live in their own namespaces. It contains no PostgreSQL clusters, object
storage configuration or credentials. The operator runs as `platform-critical`
and the Barman plugin as `platform-standard`.

## Overlay requirements

An overlay must:

- reconcile cert-manager before this base, because the Barman plugin manifest
  creates `Certificate` and `Issuer` resources;
- arrange Flux ordering so this base is ready before any `postgres-cluster`
  instance, normally with `Kustomization.spec.dependsOn`;
- reconcile this base after the one that installs the Prometheus Operator CRDs,
  because the operator chart renders a PodMonitor and the base ships a
  PodMonitor for the Barman plugin. The plugin serves its controller metrics
  over plain HTTP on 8080 (`--metrics-bind-address=:8080 --metrics-secure=false`),
  and only Prometheus may reach that port.

## Vendored Barman plugin

`plugin-barman-cloud-v0.16.0.yaml` is the release asset published by
CloudNativePG, with two changes: the plugin image and the `SIDECAR_IMAGE` Secret value it
injects into instance pods carry their multi-arch index digests. Vendoring makes reconciliation independent of GitHub availability
and keeps the base reviewable, but Renovate cannot see or update this manifest.
The kustomize patch bounds the upstream deployment resources.

To refresh the current pin, run:

```sh
version=v0.16.0
curl -fsSL "https://github.com/cloudnative-pg/plugin-barman-cloud/releases/download/$version/manifest.yaml" \
  -o "deploy/platform/cnpg/plugin-barman-cloud-$version.yaml"
digest=$(skopeo inspect --raw "docker://ghcr.io/cloudnative-pg/plugin-barman-cloud:$version" | sha256sum | cut -d' ' -f1)
sed -i "s#plugin-barman-cloud:$version\$#plugin-barman-cloud:$version@sha256:$digest#" \
  "deploy/platform/cnpg/plugin-barman-cloud-$version.yaml"
sidecar=ghcr.io/cloudnative-pg/plugin-barman-cloud-sidecar:$version
sidecar_digest=$(skopeo inspect --raw "docker://$sidecar" | sha256sum | cut -d' ' -f1)
encoded=$(printf '%s@sha256:%s' "$sidecar" "$sidecar_digest" | base64 -w0)
python3 - "deploy/platform/cnpg/plugin-barman-cloud-$version.yaml" "$encoded" <<'PY'
import re, sys
path, encoded = sys.argv[1:]
text = open(path).read()
text = re.sub(r"  SIDECAR_IMAGE: \|\n(    \S+\n)+", f"  SIDECAR_IMAGE: {encoded}\n", text)
open(path, "w").write(text)
PY
```

For an upgrade, review the new release, change both version occurrences and the
filename in `kustomization.yaml`, rerun that command with the new version, and
validate the rendered base. This manual path is the deliberate trade-off for a
pinned, self-contained upstream manifest.

