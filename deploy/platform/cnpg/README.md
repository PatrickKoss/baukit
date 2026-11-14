# CloudNativePG operator base

This Flux-native base creates `cnpg-system`, installs the CloudNativePG operator
with the `cloudnative-pg` chart pinned to `0.29.1` (operator `1.30.1`), and
installs the Barman Cloud CNPG-I plugin pinned to `v0.16.0`. Both controllers
have small, bounded resources. The plugin must run in the operator namespace and
uses cert-manager for its client and server certificates.

The operator's `POSTGRES_IMAGE_NAME` pins new clusters to
`ghcr.io/cloudnative-pg/postgresql:18.7-system-trixie` by digest. The reusable
primary and restore-test manifests reference the same image through the
`postgresql` `ImageCatalog` in the `postgres-cluster` base.

The base is intentionally cluster-wide so reusable `postgres-cluster` instances
can live in their own namespaces. It contains no PostgreSQL clusters, object
storage configuration, credentials, or priority class.

## Overlay requirements

An overlay must:

- reconcile cert-manager before this base, because the Barman plugin manifest
  creates `Certificate` and `Issuer` resources;
- arrange Flux ordering so this base is ready before any `postgres-cluster`
  instance, normally with `Kustomization.spec.dependsOn`;
- add a priority class only if the cluster policy needs one;
- decide whether to enable the operator PodMonitor after the Prometheus Operator
  CRDs exist.

## Vendored Barman plugin

`plugin-barman-cloud-v0.16.0.yaml` is the release asset published by
CloudNativePG, with one change: the plugin image carries its multi-arch index
digest. Vendoring makes reconciliation independent of GitHub availability
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
```

For an upgrade, review the new release, change both version occurrences and the
filename in `kustomization.yaml`, rerun that command with the new version, and
validate the rendered base. This manual path is the deliberate trade-off for a
pinned, self-contained upstream manifest.

