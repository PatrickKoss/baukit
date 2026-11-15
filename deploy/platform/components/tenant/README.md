# Tenant component

Turns a namespace into a tenant that Flux reconciles with tenant permissions. Use it from the
platform repository, in a kustomization that sets `namespace:` to the tenant:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: leitbild
resources:
  - namespace.yaml        # Namespace leitbild
  - tenant.yaml           # ConfigMap baukit-tenant, see below
  - sops-age.enc.yaml     # Secret sops-age-leitbild, SOPS-encrypted to the cluster recipient
  - sync.yaml             # GitRepository and Kustomization of the tenant repository
components:
  - ../../baukit/deploy/platform/components/tenant
```

The component adds:

- the PSA `restricted` labels and `baukit.dev/tenant: "true"`, which select the Kyverno tenant policies;
- ServiceAccount `flux-reconciler` (no token automount) and RoleBinding `flux-reconciler` to the
  `admin` ClusterRole in the tenant namespace only;
- `spec.serviceAccountName: flux-reconciler` on every Kustomization and HelmRelease in the build;
- `spec.targetNamespace` set to the tenant namespace and SOPS decryption with the tenant's own key on
  every Kustomization.

## Parameters

A ConfigMap named `baukit-tenant` with `config.kubernetes.io/local-config: "true"` carries them. It
is not applied. The build fails without it.

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: baukit-tenant
  annotations:
    config.kubernetes.io/local-config: "true"
data:
  sopsSecretName: sops-age-leitbild
  sopsAgeRecipient: age1...
```

`sopsSecretName` becomes `spec.decryption.secretRef.name`. `sopsAgeRecipient` is the public key; it
lands on the Namespace as the `baukit.dev/sops-age-recipient` annotation, so the tenant can look it up.

## Per-tenant SOPS key

Never give a tenant the cluster age key in `flux-system/sops-age`.

1. `age-keygen -o leitbild.agekey` creates the tenant identity. Its public key is `sopsAgeRecipient`.
2. Put the identity in a Secret named after `sopsSecretName` with the key `age.agekey`, encrypt that
   file with the cluster recipient, and commit it next to the tenant kustomization. The platform
   Kustomization in `flux-system` decrypts it with the cluster key and creates it in the tenant namespace.
3. The tenant repository's `.sops.yaml` encrypts to `sopsAgeRecipient` only. With
   `--no-cross-namespace-refs` the tenant Kustomization can read only the Secret in its own namespace.

Rotate by generating a new identity, re-encrypting the tenant repository and replacing the Secret.
