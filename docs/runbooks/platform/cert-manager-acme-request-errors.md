# BaukitCertManagerAcmeRequestErrors

## Meaning

cert-manager got HTTP 429 or 5xx answers from an ACME server for 15 minutes. 429 means a Let's Encrypt
rate limit; 5xx means the ACME server has trouble.

## Impact

New certificates and renewals stall. A tight loop of retries can also burn the rate limit for the
whole account.

## First checks

```bash
kubectl get orders,challenges -A
kubectl -n cert-manager logs deploy/cert-manager --since=30m | grep -i 'acme' | grep -iE '429|rateLimited|5[0-9][0-9]'
```

```promql
sum by (host, path, status) (increase(certmanager_http_acme_client_request_count{status=~"429|5.."}[1h]))
```

Check https://letsencrypt.status.io for a provider outage.

## Fix

- 429: find the Certificate that loops (many Orders for the same name) and fix its root cause, or
  delete the duplicate Certificate objects that request the same names. Then wait out the window in the
  error message.
- 5xx: wait for the provider. cert-manager backs off on its own.

## Escalation

Escalate to the platform owner if the account is rate limited and a certificate is within 7 days of
expiry.
