# BaukitFluxSourceFetchFailing

## Meaning

A Flux GitRepository, OCIRepository or HelmRepository has reported `Ready=False` for 10 minutes.
source-controller cannot fetch or verify the artifact.

## Impact

Every Kustomization and HelmRelease that uses this source stays on the last fetched revision. New
commits, charts and security fixes do not deploy.

## First checks

```bash
flux get sources all -A --status-selector ready=false
kubectl -n <namespace> describe <kind> <name>
kubectl -n flux-system logs deploy/source-controller --since=30m | grep '<name>'
```

```promql
gotk_resource_info{ready="False", customresource_kind=~"GitRepository|OCIRepository|HelmRepository"}
```

```logql
{namespace="flux-system", container="manager"} |= "<name>" |= "error"
```

## Fix

- Authentication failed: the deploy key or token in `spec.secretRef` expired or was rotated. Rotate the
  Secret and run `flux reconcile source git <name> -n <namespace>`.
- DNS or network: check the `flux-system` NetworkPolicy egress. The controller images have no shell, so
  start a debug pod in `flux-system` with the source-controller labels and `wget` the URL from there.
- Signature or digest verification failed: someone pushed an unsigned artifact or the trusted key
  changed. Do not disable verification; fix the signing.
- The upstream is down (GitHub, a chart repo): wait, and confirm on its status page.

## Escalation

Page the platform owner if the source of the platform itself (`flux-system/flux-system`) fails for an
hour.
