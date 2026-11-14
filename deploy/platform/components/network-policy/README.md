# Network policy components

Each platform base composes these Kustomize components and adds its own
`network-policy.yaml` with the explicit allows. Peers are named by the
`kubernetes.io/metadata.name` namespace label plus pod labels. Ports are the
pod ports, because policies match after Service translation.

| Component | Policy | Effect |
|---|---|---|
| `default-deny` | `default-deny` | Denies all ingress and egress for every pod in the namespace. |
| `dns-egress` | `allow-dns-egress` | Egress to CoreDNS pods (`k8s-app: kube-dns` in `kube-system`) on UDP and TCP 53. |
| `apiserver-egress` | `allow-apiserver-egress` | Egress to the node CIDR on 6443 and to `10.43.0.1` on 443. |
| `prometheus-scrape` | `allow-prometheus-scrape` | Ingress from Prometheus pods in `observability`. Bases patch the port and pod selector. |
| `internet-https` | `allow-internet-https` | Egress to public addresses on 443. Private ranges and `169.254.0.0/16` are excluded. Bases narrow the pod selector. |
| `node-cidr` | none | Overlay component that writes the node CIDR into policies labelled `baukit.dev/node-cidr-peer`. |

## Node CIDR

Bases ship the placeholder `192.0.2.0/32`, which matches no node, so a base
built without an overlay fails closed. An overlay composes `node-cidr` and adds
a local-config ConfigMap:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: baukit-cluster-network
  annotations:
    config.kubernetes.io/local-config: "true"
data:
  nodeCIDR: 10.89.0.0/16
```

Kustomize drops the ConfigMap from the output. If the overlay composes
`node-cidr` without it, the build fails and names `baukit-cluster-network`.
Policies labelled `egress` get the value in their first egress peer, and
policies labelled `ingress` in their first ingress peer. Webhook pods use the
ingress form, because the apiserver calls them from the node address.

The local overlay uses `10.89.0.0/16`, and `platform-up.sh` creates the k3d
network as `10.89.0.0/24` (`BAUKIT_PLATFORM_SUBNET`) so the node addresses are
known before the cluster exists. Docker otherwise picks the subnet.

## kube-router filters after DNAT

k3s enforces policies with kube-router. On k3s v1.34.8 the filter sees the
translated destination, not the Service IP. Measured on a single-node k3d
cluster from a pod whose only policy was the one listed:

| Egress allowed | Probe | Result |
|---|---|---|
| nothing | `10.43.0.1:443` | blocked |
| `10.43.0.1/32` on 443 | `10.43.0.1:443` | blocked |
| node IP on 6443 | `10.43.0.1:443` | open |
| `10.43.0.10/32` on UDP 53 | DNS lookup | blocked |
| `kube-dns` pod selector on 53 | DNS lookup | open |

So on k3s the node CIDR rule is the one that admits apiserver traffic and the
pod selector is the one that admits DNS. The `10.43.0.1` rule stays for CNIs
that filter before DNAT. It grants nothing on k3s. No Service IP rule exists
for DNS.
