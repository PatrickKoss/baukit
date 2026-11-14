# BaukitCertManagerCertificateExpiringSoon

## Meaning

A cert-manager Certificate expires in under 21 days. cert-manager renews at two thirds of the
lifetime, so for a 90-day ACME certificate the renewal is about nine days late.

## Impact

Nothing breaks yet. If renewal keeps failing, `BaukitCertManagerCertificateExpiryCritical` fires at
7 days and clients fail TLS on expiry.

## First checks

```bash
kubectl -n <namespace> get certificate <name> -o wide
kubectl -n <namespace> describe certificate <name>
kubectl -n <namespace> get certificaterequests,orders,challenges
kubectl -n cert-manager logs deploy/cert-manager --since=1h | grep '<namespace>/<name>'
```

```promql
(certmanager_certificate_expiration_timestamp_seconds - time()) / 86400
certmanager_certificate_renewal_timestamp_seconds - time()
```

## Fix

- A Challenge is stuck: read its `status.reason`. For HTTP-01 check that Traefik routes
  `/.well-known/acme-challenge/` to the solver pod; for DNS-01 check the provider token.
- An Order failed with a rate limit: wait for the window named in the error, then
  `cmctl renew <name> -n <namespace>`.
- The issuer is not Ready: `kubectl get clusterissuer,issuer -A` and fix its Secret or account key.

## Escalation

Escalate to the platform owner if renewal still fails with 10 days left.
