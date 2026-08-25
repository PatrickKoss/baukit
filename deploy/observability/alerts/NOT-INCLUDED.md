# Alerts owned outside this application pack

These placeholders from analysis section 8.6 require infrastructure, exporter, or product-specific signals that are outside the application telemetry specification:

- Crash loops and pod restarts — define from `kube-state-metrics` in the `platform-infra` alert bundle.
- OOM kills — define from Kubernetes container termination metrics in `platform-infra`.
- CPU throttling and memory pressure — define from kubelet/cAdvisor and node metrics in `platform-infra`.
- Persistent-volume and disk pressure — define from kubelet volume and node filesystem metrics in `platform-infra`.
- Stale or failed backups and restore-test age — define from CloudNativePG and backup-controller metrics in `platform-infra`.
- Certificate expiry — define from cert-manager metrics in `platform-infra`.
- PostgreSQL connection failures beyond application acquire timeouts — define from database/exporter metrics in `platform-infra`.
- External synthetic checks — define in the platform probe service and alert from its probe metrics in `platform-infra`.
- Offline-sync failures and lag — define in each product after it publishes bounded product-owned metrics, then add product alert rules beside `platform-infra`.
