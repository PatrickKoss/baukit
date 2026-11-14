# BaukitCertManagerCertificateExpiryCritical

## Meaning

A cert-manager Certificate expires in under 7 days. Renewal has failed for weeks.

## Impact

Clients reject the certificate when it expires. For the ingress wildcard that takes every product
offline over HTTPS.

## First checks

```bash
kubectl -n <namespace> describe certificate <name>
kubectl -n <namespace> get certificaterequests,orders,challenges -o wide
kubectl get clusterissuer -o wide
```

```promql
(certmanager_certificate_expiration_timestamp_seconds{namespace="<namespace>", name="<name>"} - time()) / 86400
increase(certmanager_http_acme_client_request_count{status=~"4..|5.."}[1h])
```

## Fix

Work through `cert-manager-certificate-expiring-soon` first. If ACME stays blocked, issue a
certificate by hand from another ACME account or CA, store it in the same Secret name, and keep the
Certificate object so cert-manager takes over once the issuer works again.

## Escalation

Page the platform owner now. With under 2 days left, tell product owners which hosts are affected.
