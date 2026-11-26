# ADR 0030: The platform is a menu of Flux Kustomize bases for single-node k3s, tested on a local k3d cluster, with one tenant namespace per product environment

## Status

Accepted, 2026-11-27.

## Context

Products need somewhere to run: ingress, certificates, PostgreSQL, Redis,
observability, admission policy and runtime security. Each product building
its own cluster setup would repeat the work and drift. A complete distribution
would force every cluster to run components it does not need.

The first clusters are small single nodes, and development runs on a WSL2
host. Choices that need several nodes or kernel modules cost more than they
buy there. Paid test environments are not wanted, so the stack has to come up
and pass its checks on a local cluster.

## Decision

`deploy/platform/` holds independently includable Flux Kustomize bases, as
[`deploy/platform/README.md`](../../deploy/platform/README.md) describes. Each
first-level base owns its namespace and its chart sources. HelmRelease chart
versions are exact pins. Overlays under `deploy/platform/overlays/`
(`local`, `everything`, `baremetal`) compose the bases they need. Identity and
secrets stay in the private infrastructure repository.

`deploy/platform/platform-up.sh` creates a local k3d cluster and reconciles an
overlay through Flux from a local git server. Products run in tenant
namespaces built with `components/tenant`, which gives each tenant its own
reconciler ServiceAccount and runs under PSA `restricted`.

These component choices hold today:

- Networking uses k3s' built-in kube-router NetworkPolicy controller, not
  Cilium. Internet egress is an `ipBlock` rule.
- Falco runs with `driver.kind: modern_ebpf` and no kernel module. Events go
  through falcosidekick to Loki and Alertmanager. The web UI is off.
- Kyverno policies use the CEL types `ValidatingPolicy`, `GeneratingPolicy` and
  `ImageValidatingPolicy`. No `ClusterPolicy` remains.
- No policy mutates workloads. `generate-tenant-defaults` only generates
  default objects in tenant namespaces.
- `verify-baukit-images` denies tenant images from the product registry that
  are not pinned by digest or not signed with the cosign key in
  `kyverno/baukit-image-signing-key`. Signatures skip the public Rekor log.
- Traefik runs as a Deployment behind k3s ServiceLB (`type: LoadBalancer`), with
  no hostPort, so its namespace runs PSA `restricted`.
- PostHog stays on its self-host chart, fenced by NetworkPolicy and PSA
  `baseline`.

The gate lane for the cluster test is [ADR 0014](0014-one-quality-gate.md).

## Enforced by

- The `platform-validate` step runs `deploy/platform/validate.sh`. It renders
  every base and overlay, checks them with kubeconform against schemas built
  from each CRD, and runs the convention checks in
  `deploy/platform/lint/check_conventions.py` with the active rules in
  `deploy/platform/lint/conventions.toml`, among them `psa-labels`,
  `default-deny-network-policy`, `image-digest`, `priority-class` and
  `pdb-multi-replica`. It also runs the Kyverno CLI tests under
  `deploy/platform/kyverno-policies/tests/`.
- The `platform-e2e` step runs `deploy/platform/e2e/run.sh`, which brings the
  stack up on a throwaway k3d cluster and runs chainsaw and admission tests
  such as `deploy/platform/e2e/test_admission_cases.py`.
- Nothing rejects a new `ClusterPolicy`, a mutating policy or Cilium. A
  convention rule over the rendered output would.

## Consequences

Clusters run only the bases they include, and a product gets a namespace with
policy, network isolation and monitoring already set. The stack targets one
node. FQDN egress, multi-node failover and a PostHog replacement are not
covered. Signing images is the product's job before a tenant can run them.
The local cluster test takes minutes and needs Docker.

## History

- 2026-11-14, `11a53a13`: the Kyverno CEL policy library replaced the
  `ClusterPolicy` objects.
- 2026-11-14, `55473e22`: Traefik moved from a DaemonSet with hostPorts to a
  Deployment behind ServiceLB.
- 2026-11-17, `cc4f17d1`: key-based cosign verification of product images
  became enforced.
