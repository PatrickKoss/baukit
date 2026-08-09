# CloudNativePG operator base

This Flux-native base creates `cnpg-system`, installs the CloudNativePG operator
with the `cloudnative-pg` chart pinned to `0.29.0` (operator `1.30.0`), and
installs the Barman Cloud CNPG-I plugin pinned to `v0.14.0`. Both controllers
have small, bounded resources. The plugin must run in the operator namespace and
uses cert-manager for its client and server certificates.

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

`plugin-barman-cloud-v0.14.0.yaml` is the exact release asset published by
CloudNativePG. Vendoring makes reconciliation independent of GitHub availability
and keeps the base reviewable, but Renovate cannot see or update this manifest.
The kustomize patch bounds the upstream deployment resources without changing
the vendored bytes.

To refresh the current pin exactly, run:

```sh
curl -fsSL https://github.com/cloudnative-pg/plugin-barman-cloud/releases/download/v0.14.0/manifest.yaml \
  -o deploy/platform/cnpg/plugin-barman-cloud-v0.14.0.yaml
```

For an upgrade, review the new release, change both version occurrences and the
filename in `kustomization.yaml`, rerun that command with the new version, and
validate the rendered base. This manual path is the deliberate trade-off for a
pinned, self-contained upstream manifest.

