# Hetzner Cloud provider

This optional provider base installs the Hetzner Cloud Controller Manager chart
`1.38.0` and hcloud CSI chart `2.23.0` in `kube-system`. The CSI driver provides
the default `hcloud-volumes` StorageClass with expansion, delayed binding, and a
conservative `Retain` reclaim policy, plus `hcloud-volumes-delete` with a
`Delete` reclaim policy for throwaway volumes. The Postgres restore test uses
it: set `RESTORE_STORAGE_CLASS=hcloud-volumes-delete` on the
`postgres-restore-test` CronJob in the Hetzner overlay, otherwise every monthly
run leaves a retained 10 GiB volume behind.

The CCM is present for provider node metadata. Its Load Balancer controller is
off (`HCLOUD_LOAD_BALANCERS_ENABLED=false`): the Traefik `LoadBalancer` Service
is answered by k3s ServiceLB on the node, and a managed Hetzner Load Balancer
would be a second, billed answer to the same Service. Remove that variable
only together with disabling ServiceLB.

The base manages no Namespace object. `kube-system` belongs to k3s, and a
Flux-managed copy would be pruned (deleted) when the base is removed. CSI-backed Hetzner Volumes are the useful storage part
of this base, and are not backups.

The overlay **must** create a Secret named `hcloud` in `kube-system` with key
`token`. Both charts read that same Secret; the base ships no Secret or token.
The token needs only the cloud permissions required by the enabled components.
The overlay should also remove k3s `local-path` as the default StorageClass (or
patch `hcloud-volumes` to non-default) so the cluster has exactly one default.
Both charts render their monitors (a PodMonitor for the CCM on 8233, a
ServiceMonitor for the CSI driver), so reconcile this base after the one that
installs the Prometheus Operator CRDs. The CSI controller and node pods run as
`platform-critical`, since volumes must attach before PostgreSQL can recover.
The CCM keeps the chart's `system-cluster-critical`.

The local k3d flavor must omit this entire base. It is also optional on a
Hetzner cluster that uses neither Cloud Volumes nor a managed Load Balancer.
