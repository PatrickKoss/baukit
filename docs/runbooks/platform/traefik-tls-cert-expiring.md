# BaukitTraefikTlsCertExpiring

## Meaning

A certificate Traefik serves expires in under 7 days. Traefik reports what it loaded, so this catches
the case where cert-manager renewed the Secret but Traefik still serves the old one.

## Impact

Clients fail TLS for that host when the certificate expires.

## First checks

```bash
echo | openssl s_client -connect <host>:443 -servername <cn> 2>/dev/null | openssl x509 -noout -enddate
kubectl get certificates -A -o wide | grep '<cn>'
kubectl -n traefik logs deploy/traefik --since=1h | grep -i tls
```

```promql
(traefik_tls_certs_not_after - time()) / 86400
certmanager_certificate_expiration_timestamp_seconds - time()
```

## Fix

- cert-manager has not renewed: follow `cert-manager-certificate-expiry-critical`.
- cert-manager renewed but Traefik serves the old one: check that the IngressRoute or TLSStore points at
  the right Secret, then `kubectl -n traefik rollout restart deploy/traefik`.

## Escalation

Page the platform owner. With under 2 days left, tell product owners which hosts are affected.
