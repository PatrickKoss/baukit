# cert-manager

This base installs the Jetstack `cert-manager` chart at `v1.21.1` in the
`cert-manager` namespace. The chart installs and retains its CRDs. Controller,
webhook, CA injector, and startup-check resources are bounded for a small
single-node cluster; Prometheus-format metrics remain enabled.

An overlay must create the ClusterIssuers and supply their identity-bearing
configuration. The conventional names are `letsencrypt-staging` and
`letsencrypt-prod`. The overlay owns the ACME e-mail, ACME server choice,
solver/DNS provider configuration, and any credential Secrets. Start with the
staging issuer and prove issuance before selecting the production issuer.

This base deliberately creates no Issuer, ClusterIssuer, Certificate, domain,
e-mail address, or Secret.
