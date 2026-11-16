# Cluster baseline

This base creates the `platform-system` namespace and the non-default
`platform-critical` and `platform-standard` PriorityClasses. Every other base
sets one of them on each pod it renders, and the `priority-class` convention
rule fails a pod without one. k3s runs CoreDNS, local-path-provisioner and
metrics-server in `kube-system` with its own `system-cluster-critical` class,
which ranks above both.

`app-namespace-policy/` is a Kustomize component for an application namespace.
An overlay must include it from a Kustomization that sets `namespace`, then add
the application's explicit ingress and egress allow policies. Patch the quota
from measured needs when the defaults (1 requested CPU/2 GiB requested memory,
2 CPU/4 GiB limits, 20 pods, and 10 PVCs) are unsuitable. The component is
deliberately not part of this base: applying a blanket default-deny policy to
namespaces the base does not own would interrupt workloads before their allows
exist.

Example overlay fragment:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: my-application
resources:
  - namespace.yaml
  - application-allow-policies.yaml
components:
  - ../../../../baukit/deploy/platform/cluster-baseline/app-namespace-policy
```

The base contains no secrets or cluster identity. Application namespaces and
all NetworkPolicy allows remain overlay or application data.
