# falco

Runtime security for the platform: Falco 0.45.2 (chart `falcosecurity/falco` 9.2.2) as a DaemonSet
with the `modern_ebpf` driver, plus one falcosidekick replica that forwards events to Loki and to
Alertmanager (priority Warning and above). Images are pinned by digest. Falco, falcosidekick and
the falcosidekick Helm test Pod run as `platform-standard`; the test Pod gets it from a
`postRenderers` patch because the subchart has no key for it.

Falco reads container and pod metadata from the k3s containerd socket
(`/run/k3s/containerd/containerd.sock`). The upstream rules and the container plugin come from the image,
so no falcoctl sidecar fetches artifacts at runtime.

## Rules

`rules/baukit.rules.yaml` becomes the `falco-baukit-rules` ConfigMap, mounted at `/etc/falco/rules.d`
next to the upstream `falco_rules.yaml`. It adds the baukit app and platform rules and the noise macros
from the FA0 spike. Falco reloads the directory when the ConfigMap changes.

Validate rules locally with `./validate-rules.sh` (Docker, no network). `validate.sh` runs it, so a broken
rule fails `scripts/ci-gate.sh --step platform-validate`.

## Overlay contract

An overlay may add a ConfigMap `falco-overlay-rules` in namespace `falco`. Its files load after
`baukit.rules.yaml`. A node with a dedicated kernel (no Docker or other clusters on the host) should
override the shared-kernel filter there, so host processes are watched again:

```yaml
- macro: baukit_shared_kernel
  condition: (never_true)
  override:
    condition: replace
```

Add an exception as a macro with a comment that names where the noise came from, append it to the
rule with `override: {condition: append}`, and keep it as narrow as one image or one process.

## Network

Default deny in the namespace. Prometheus may scrape 8765 (Falco) and 2801 (falcosidekick). Falco may
reach falcosidekick on 2801, and so may pods with the falcosidekick labels, which the chart's Helm test
pod carries. Falcosidekick may reach itself on 2801, `loki-gateway` (8080) and Alertmanager (9093) in
`observability`, plus DNS.

Alerts, the dashboard and the runbooks live in `deploy/observability-platform` (component `falco`).
