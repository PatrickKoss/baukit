# falco

Runtime security for the platform: Falco 0.45.2 (chart `falcosecurity/falco` 9.2.2) as a DaemonSet
with the `modern_ebpf` driver, plus one falcosidekick replica that forwards events to Loki and to
Alertmanager (priority Warning and above). Images are pinned by digest. Falco, falcosidekick and
the falcosidekick Helm test Pod run as `platform-standard`; the test Pod gets it from a
`postRenderers` patch because the subchart has no key for it.

Falco reads container and pod metadata from the k3s containerd socket
(`/run/k3s/containerd/containerd.sock`). The upstream rules and the container plugin come from the image,
so no falcoctl sidecar fetches artifacts at runtime.

A second release, `falco-k8saudit`, runs the same chart and image as a single-replica Deployment
without a driver. Its k8saudit plugin listens on 9765 for the API server's audit webhook, which reaches
it on the fixed ClusterIP `10.43.0.60` (Service `falco-k8saudit-webhook`) because the API server cannot
resolve cluster DNS. The address sits in the static band of the k3s default service CIDR
`10.43.0.0/16`; a cluster with another service CIDR changes the release and `../k3s/audit-webhook.yaml`
together. The plugin is not in the image, so a falcoctl init container installs
`ghcr.io/falcosecurity/plugins/plugin/k8saudit` by digest and checks its cosign signature. That pod runs
as 65534 with a read-only root, no capabilities and no service account token, and sends its events to
the same falcosidekick, so they reach Loki and Alertmanager like the syscall events (`eventsource`
label `k8s_audit`).

## Rules

`rules/baukit.rules.yaml` becomes the `falco-baukit-rules` ConfigMap, mounted at `/etc/falco/rules.d`
next to the upstream `falco_rules.yaml`. It adds the baukit app and platform rules and the noise macros
from the FA0 spike. Falco reloads the directory when the ConfigMap changes.

`k8saudit-rules/baukit.k8saudit.rules.yaml` becomes `falco-k8saudit-rules` for the k8saudit release:

| Rule | Priority | Fires on |
|---|---|---|
| Baukit secret read by unexpected service account | Warning | get, list or watch on Secrets by a service account outside `baukit_expected_secret_reader` (flux-system, kube-system, kyverno, cert-manager, cnpg-system, observability, traefik, and service accounts in `postgres` reading Secrets in `postgres`, which CNPG instances do) |
| Baukit exec or attach into pod | Warning | create (SPDY) or get (WebSocket, kubectl since 1.30) on `pods/exec` or `pods/attach`, except `baukit_expected_exec`: execs into `postgres` by the restore-test CronJob and the CNPG operator |
| Baukit ClusterRoleBinding changed outside GitOps | Critical | create, update, patch or delete of a ClusterRoleBinding by anyone but Flux's kustomize- and helm-controller or k3s, dry runs excluded |

An overlay adds an expected secret reader with
`- macro: baukit_expected_secret_reader` and `override: condition: append` in a rules file of its own
mounted into the k8saudit release.

Validate rules locally with `./validate-rules.sh` (Docker). The syscall rules run without network; the
k8saudit rules need the plugin, which the script downloads once per digest with the pinned falcoctl into
`${XDG_CACHE_HOME:-~/.cache}/baukit/falco-plugins`. `./validate-rules.sh --k8saudit FILE` checks one
audit rules file, and `./validate-rules.sh --k8saudit-replay EVENTS` runs the shipped audit rules over a
file of audit events (one JSON event per line) and prints each alert. `validate.sh` runs it, so a broken
rule fails `scripts/ci-gate.sh --step platform-validate`.

## Overlay contract

The base owns the HelmRelease, the `falco-baukit-rules` ConfigMap and the network policies. An
overlay changes Falco through two things only:

- a ConfigMap `falco-overlay-rules` in namespace `falco`. The projected volume marks it optional, so
  the base runs without it. Its files load after `baukit.rules.yaml`, so they can override any list,
  macro or rule from upstream or from baukit;
- the receivers of the `alertmanager-routing` Secret. Falcosidekick posts every Warning+ event to
  Alertmanager with `component: falco`, and `BaukitFalcoCriticalEvent` carries the same label, so
  route `component=falco` to whoever answers security pages.

Do not patch `falco_rules.yaml` or the HelmRelease values in an overlay. A new rule that every product
needs belongs in `baukit.rules.yaml`.

`baukit_shared_kernel` defaults to `never_true`, so the base watches host processes too. A cluster that
shares its kernel with Docker, other k3d clusters or gate containers (k3d on WSL2, the e2e cluster)
adds the component `../components/falco-shared-kernel`. Its `falco-overlay-rules` ConfigMap turns the
macro on, and the baukit rules then drop events that carry no `k8s.ns.name`. A production overlay
needs nothing.

## Noise tuning

A rule that fires on normal operation trains people to ignore Falco, so every Warning+ event on an
idle cluster is a bug. The last step of `e2e/tests/falco` fails on any Warning+ event since Falco
started, covering platform install and the suites that run alongside it, except from the pods the
tests trigger on purpose. To tune a noisy rule:

1. Find the events and the fields that set them apart:

   ```bash
   kubectl -n falco logs ds/falco -c falco --since=1h | grep '"priority":"Warning"'
   ```

   In Grafana, `{source="syscall", priority=~"Warning|Error|Critical"} | json` shows the same events
   with `k8s_ns_name` and `k8s_pod_name`.
2. Decide whether the activity is expected. If nobody can explain it, it is not noise: follow the
   runbook of the alert.
3. Write the narrowest exception that matches it: one image repository, one process name, one path.
   Never exclude a whole namespace or a whole rule.
4. Validate with `./validate-rules.sh`, then rerun the falco e2e suite
   (`deploy/platform/e2e/run.sh --suite tests/falco`) and check that the noise step passes and
   the detection steps still fire.

## Adding an exception macro

Exceptions are macros, appended to the upstream rule with `override`. The comment above the macro
names where the noise came from (the run, the date, the process) so a later reader can tell whether
the exception still applies:

```yaml
# backup-agent reads /etc/shadow to copy it into node backups (ops ticket 123, 2026-11-20).
- macro: baukit_backup_agent_reads_shadow
  condition: (k8s.ns.name = backups and container.image.repository = registry.example.com/backup-agent and proc.name = backup-agent)

- rule: Read sensitive file untrusted
  condition: and not baukit_backup_agent_reads_shadow
  override:
    condition: append
```

Platform-wide exceptions go into `rules/baukit.rules.yaml` and are listed in `baukit_known_noise`.
Exceptions for one deployment go into the overlay's `falco-overlay-rules` ConfigMap. Rerun
`./validate-rules.sh` after either change.

## Network

Default deny in the namespace. Prometheus may scrape 8765 (Falco) and 2801 (falcosidekick). Falco may
reach falcosidekick on 2801, and so may pods with the falcosidekick labels, which the chart's Helm test
pod carries. Falcosidekick may reach itself on 2801, `loki-gateway` (8080) and Alertmanager (9093) in
`observability`, plus DNS. The k8saudit pod accepts 9765 from the node CIDR only (the API server calls
from the node address; `allow-k8saudit-webhook-from-nodes` carries the `node-cidr-peer` placeholder, so
an overlay must compose `components/network-policy/node-cidr`), and may reach public addresses on 443 for
the plugin download (`allow-internet-https`, narrowed to that pod).

Alerts, the dashboard and the runbooks live in `deploy/observability-platform` (component `falco`).
The runbooks are in `docs/runbooks/platform/falco-*.md` and `falcosidekick-output-errors.md`. Idle
resource use is in `../local-footprint.md`.
