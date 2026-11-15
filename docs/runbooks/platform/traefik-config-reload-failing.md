# BaukitTraefikConfigReloadFailing

## Meaning

A Traefik Kubernetes provider (`kubernetescrd` or `kubernetes`) has logged `Provider error, retrying in`
for 10 minutes. Traefik v3 has no metric for this, so a Loki ruler rule counts the log line.

## Impact

Traefik keeps routing with the last configuration it built. New or changed IngressRoutes, Ingresses,
Middlewares and TLS options do not take effect until the provider recovers.

## First checks

```bash
kubectl -n traefik logs deploy/traefik --since=30m | grep 'Provider error'
kubectl auth can-i list ingressroutes.traefik.io --as=system:serviceaccount:traefik:traefik -A
kubectl get crd | grep traefik.io
```

```logql
{namespace="traefik"} |= "Provider error, retrying in"
```

## Fix

- `forbidden`: the Traefik ClusterRole lost a verb or resource. Reconcile the Traefik HelmRelease with
  `flux reconcile helmrelease traefik -n traefik`.
- `the server could not find the requested resource`: a Traefik CRD is missing or at the wrong version.
  Reconcile the HelmRelease, which installs the CRDs.
- API server timeouts: check the API server and the `traefik` egress NetworkPolicy to the API server.

## Escalation

Page the platform owner if routes must change while the provider is down.
