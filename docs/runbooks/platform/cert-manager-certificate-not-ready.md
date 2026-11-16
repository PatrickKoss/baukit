# BaukitCertManagerCertificateNotReady

## Meaning

A Certificate has had its `Ready` condition at `False` or `Unknown` for 10 minutes. Its Secret is
missing, does not match the spec, or a renewal is in flight and failing.

## Impact

A new host may have no certificate at all, so Traefik serves its default certificate and browsers show
an error. An existing host keeps its old certificate until it expires.

## First checks

```bash
kubectl -n <namespace> describe certificate <name>
kubectl -n <namespace> get secret <secretName> -o jsonpath='{.metadata.annotations}'
kubectl -n <namespace> get certificaterequest -l cert-manager.io/certificate-name=<name>
```

```promql
certmanager_certificate_ready_status{condition!="True"} == 1
```

```logql
{namespace="cert-manager"} |= "<name>" |= "error"
```

## Fix

- `DoesNotExist` or `IncorrectIssuer`: the issuer name in the Certificate is wrong or the issuer was
  deleted. Fix it in Git.
- A CertificateRequest is `Denied`: an approver policy rejected it; read the reason.
- A Challenge does not pass: see `cert-manager-certificate-expiring-soon` for HTTP-01 and DNS-01 checks.

## Escalation

Escalate to the platform owner if it stays not Ready for an hour or affects a production host.
