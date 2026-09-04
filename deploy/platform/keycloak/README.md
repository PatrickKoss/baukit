# Keycloak identity base

This base creates `keycloak`, installs the official namespace-scoped Keycloak
Operator pinned to `26.7.1` without OLM, and declares one production-mode,
single-instance `Keycloak` resource with bounded resources. The upstream
namespace-scoped kustomize resources are vendored under
`vendor/keycloak-operator-26.7.1/`; preview cluster-wide resources are not used.

The Keycloak CR expects the shared CNPG convention: database `keycloak`, owner
`keycloak_owner`, generated service host
`postgres-cluster-rw.postgres.svc.cluster.local`, and credentials from
`keycloak-db-credentials`. The Secret is not shipped.

## Overlay requirements and dependencies

An overlay must:

- reconcile `cnpg` and one `postgres-cluster` instance before this base, using
  Flux `Kustomization.spec.dependsOn` rather than a cross-base kustomize
  reference;
- add the `keycloak`/`keycloak_owner` `Database` and `DatabaseRole` pair to its
  PostgreSQL instance and create `keycloak-db-credentials` in both consumers'
  namespace as needed (or patch the Secret/host convention);
- create `keycloak-bootstrap-admin` with `username` and `password` keys; it is
  consumed only during initial bootstrap and should be rotated/removed according
  to the operator runbook afterward;
- patch `spec.hostname.hostname`, `spec.http.tlsSecret`, and the desired
  `spec.ingress` settings. The base leaves ingress disabled and supplies no
  domain or TLS Secret. Enabling plain HTTP is not the production convention;
- add a priority class, ingress annotations/class, proxy headers, and measured
  resource adjustments when the cluster overlay requires them.

The operator runs Keycloak with its production start command. This base is not
expected to become Ready until the overlay supplies database credentials,
hostname, and TLS.

## Accessible login theme packaging

The generated development bind mount is not suitable for this Operator base. Build an immutable Keycloak image that copies `keycloak/themes/baukit-accessible` into `/opt/keycloak/themes/baukit-accessible`, then patch `spec.image` to the image digest. A product child belongs in the same image under `/opt/keycloak/themes/PRODUCT`. Keep the base theme unchanged and put product CSS and message bundles in that child.

Use the same Keycloak `26.7.1` base image as this Operator pin. A minimal image recipe is:

```Dockerfile
FROM quay.io/keycloak/keycloak:26.7.1
COPY --chown=keycloak:keycloak keycloak/themes/baukit-accessible /opt/keycloak/themes/baukit-accessible
COPY --chown=keycloak:keycloak keycloak/themes/PRODUCT /opt/keycloak/themes/PRODUCT
```

Build and publish that image through the product's release pipeline, resolve its digest, and patch the private overlay:

```yaml
apiVersion: k8s.keycloak.org/v2beta1
kind: Keycloak
metadata:
  name: keycloak
spec:
  image: registry.example.invalid/keycloak-product@sha256:DIGEST_REQUIRED
```

Run the Baukit theme browser suite against the final image before rollout. After the new pods are Ready, apply an ordered realm migration that sets `loginTheme` to `baukit-accessible` or the product child. Rollback restores the previous image digest and previous realm setting. Do not package the theme through a mutable tag, writable persistent volume, or ad hoc ConfigMap mount. Every Keycloak patch or minor upgrade requires an inherited-markup review and the full browser suite.

## Realm as code

Use exactly one realm per product. Copy `realm-import.example.yaml` into the
private overlay, commit the full Git-owned `RealmRepresentation`, and apply its
`KeycloakRealmImport` once for initial creation. Imports are create-only:
editing or deleting the CR does not update or delete the live realm.

Every later realm change is an explicit, ordered Admin API migration Job. Copy
`realm-migration-job.example.yaml`, assign the next immutable sequence, pin the
migration image by digest, keep its operation idempotent, and record completion
in the migration implementation. The overlay supplies a least-privilege admin
API service-account Secret named `keycloak-realm-migrator`. The skeleton is
commented out because the first executable migration and its image land with
the first real realm change in a later wave.

## Vendored operator upgrade

Vendoring keeps bootstrap independent of upstream availability but is invisible
to Renovate. To refresh the pinned upstream directory exactly, download the six
files from the versioned tag:

```sh
KEYCLOAK_VERSION=26.7.1
for file in keycloakoidcclients.k8s.keycloak.org-v1.yml \
  keycloakrealmimports.k8s.keycloak.org-v1.yml \
  keycloaks.k8s.keycloak.org-v1.yml \
  keycloaksamlclients.k8s.keycloak.org-v1.yml kubernetes.yml kustomization.yml; do
  curl -fsSL \
    "https://raw.githubusercontent.com/keycloak/keycloak-k8s-resources/${KEYCLOAK_VERSION}/kubernetes/${file}" \
    -o "deploy/platform/keycloak/vendor/keycloak-operator-${KEYCLOAK_VERSION}/${file}"
done
```

For an upgrade, change the version once, review the upstream diff and release
notes, rename the vendor directory/reference, then rerun all base validation.
