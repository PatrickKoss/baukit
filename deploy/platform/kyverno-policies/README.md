# Kyverno policies

The baukit policy library. It needs the `kyverno` base installed first (CRDs and controllers) and a
ConfigMap `kyverno/baukit-registry-allowlist` that the overlay owns. Its `registries` key holds one
image prefix per line, for example `ghcr.io/`. Without it every image fails the registry policies,
so a tenant namespace admits no Pods.

All policies use the Kyverno 1.20 CEL types (`ValidatingPolicy`, `ImageValidatingPolicy`,
`GeneratingPolicy`). Tenant namespaces are the ones labelled `baukit.dev/tenant=true`; include
`../components/tenant` in the namespace's kustomization to get that label and the PSA labels.

| Policy | Applies to | Mode |
|---|---|---|
| `require-container-resources` | all Pods | Audit |
| `disallow-latest-image-tag` | all Pods | Audit |
| `protect-operations-ports` | public Services, Ingresses | Audit |
| `restrict-image-registries` | Pods outside tenants | Audit |
| `restrict-image-registries-tenants` | tenant Pods | Deny |
| `require-image-digest` | tenants labelled `baukit.dev/environment=production` | Audit |
| `require-probes` | tenant Deployments and StatefulSets | Deny |
| `require-baukit-labels` | tenant workloads | Deny |
| `disallow-default-namespace` | all workloads and Ingresses | Deny |
| `restrict-service-external-ips` | all Services | Deny |
| `restrict-loadbalancer-and-nodeport` | all Services | Deny |
| `unique-ingress-host-and-path` | all Ingresses | Deny |
| `disallow-sa-token-automount` | tenant Pods | Deny |
| `restrict-wildcard-rbac` | Roles and ClusterRoles outside system namespaces | Audit |
| `restrict-cluster-admin-binding` | all bindings to `cluster-admin` | Deny |
| `require-psa-labels` | Namespaces | Deny |
| `restrict-netpol-wildcards` | tenant NetworkPolicies | Deny |
| `flux-tenant-service-account` | tenant Flux Kustomizations and HelmReleases | Deny |
| `verify-baukit-images` | tenant Pods with `ghcr.io/patrickkoss/*` images | Audit |
| `generate-tenant-defaults` | tenant Namespaces | generates, synchronized |

`ValidatingPolicy` has one `validationActions` list per policy, so "Audit everywhere, Deny in
tenants" is two policies with complementary namespace selectors.

`require-psa-labels` allows `privileged` only for `kube-system`, `observability-agents`, `kured` and
`falco`, and requires `restricted` on tenant namespaces.

`generate-tenant-defaults` gives every tenant namespace a `default-deny` NetworkPolicy, an
`allow-dns-egress` NetworkPolicy to CoreDNS and a `default-limits` LimitRange. It also runs for
namespaces that existed before the policy.

## Pod Security Standards

`pod-security-release.yaml` installs the upstream `kyverno-policies` chart `3.10.0` with the
`restricted` standard at `v1.34` in Audit mode cluster-wide, so PolicyReports show restricted gaps in
every namespace. The chart renders each policy once with a fixed name and one action, and its values
offer no namespace selector, so it cannot also Deny in tenants. Tenants get enforcement from Pod
Security Admission instead: the tenant component sets `pod-security.kubernetes.io/enforce:
restricted`, and `require-psa-labels` rejects a tenant namespace without it.

## Exceptions

`exceptions/` holds `PolicyException` objects (`policies.kyverno.io`). They must live in the
`kyverno` namespace and carry a `baukit.dev/reason` annotation naming the invariant.

## Tests

Each policy has `tests/<name>/kyverno-test.yaml` with at least one passing and one failing resource:

```bash
kyverno test deploy/platform/kyverno-policies/tests/
```

`validate.sh` runs these suites and applies the policies to every render; see
`../lint/ci-policies/README.md`.

Expected `skip` results do not bite: the CLI reports an excluded resource as matching any
expectation. The pass and fail rows do.

`verify-baukit-images` has no signed pass fixture yet. Its fail fixture is a tag-only product
image, which fails offline because verification needs a digest. A signed case needs product images in
a registry (plan D6).
