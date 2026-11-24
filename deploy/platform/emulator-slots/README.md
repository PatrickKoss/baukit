# Emulator slots

This optional base runs squat's generic device plugin, pinned by digest, as the DaemonSet
`kube-system/emulator-slots`. It advertises the host's `/dev/kvm` as the extended resource
`squat.ai/kvm`, `devicesPerNode` times on each node that carries the label `nodeLabelKey=nodeLabelValue`.
A Pod that requests `squat.ai/kvm: 1` gets `/dev/kvm` and one slot. The scheduler leaves the next
Pod Pending once every slot on the selected nodes is taken. It is inert unless a cluster overlay composes it.

The base uses the defaults in `parameters.yaml`: label `baukit.dev/emulators=true` and two devices
per node. To change them, compose `../components/emulator-slots` with your own `ConfigMap
emulator-slots`. Mark it `config.kubernetes.io/local-config: "true"` and set all three keys:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - parameters.yaml   # ConfigMap emulator-slots: nodeLabelKey, nodeLabelValue, devicesPerNode
components:
  - <baukit>/deploy/platform/components/emulator-slots
```

The plugin runs as root without privileges or capabilities, read-only, and mounts only the kubelet's
device-plugin directory and `/dev/kvm`. The PolicyException `kyverno/emulator-slots` admits those
hostPath volumes and the root user for this DaemonSet and its Pods only. `/dev/kvm` keeps its host
group, so emulator Pods add that GID to `supplementalGroups`.

The cluster operator labels the nodes. Two slots per node fits a node with room for two emulators.
A slot does not reserve memory, so the emulator Pod's own requests do that.
