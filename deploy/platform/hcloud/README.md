# Hetzner Cloud provider

This optional provider base installs the Hetzner Cloud Controller Manager chart
`1.34.0` and hcloud CSI chart `2.22.1` in `kube-system`. The CSI driver provides
the default `hcloud-volumes` StorageClass with expansion, delayed binding, and a
conservative `Retain` reclaim policy. The CCM is present for provider node
metadata and is the integration needed if a Hetzner `LoadBalancer` Service is
adopted later; the initial platform uses Traefik hostPorts and does not create a
managed Load Balancer. CSI-backed Hetzner Volumes are the useful storage part
of this base, and are not backups.

The overlay **must** create a Secret named `hcloud` in `kube-system` with key
`token`. Both charts read that same Secret; the base ships no Secret or token.
The token needs only the cloud permissions required by the enabled components.
The overlay should also remove k3s `local-path` as the default StorageClass (or
patch `hcloud-volumes` to non-default) so the cluster has exactly one default.
Enable the charts' PodMonitor/ServiceMonitor values only after the Prometheus
Operator CRDs exist, and set a `priorityClassName` if desired.

The local k3d flavor must omit this entire base. It is also optional on a
Hetzner cluster that uses neither Cloud Volumes nor a managed Load Balancer.
