# Traefik

This base installs Traefik chart `42.4.1` (Traefik `v3.7.19`) as a Deployment
in the `traefik` namespace behind a `LoadBalancer` Service. On k3s, ServiceLB
(klipper-lb) answers that Service: its `svclb-traefik` pods in `kube-system`
hold host ports 80 and 443 and forward to the Traefik pods. Traefik itself uses
no hostPort and no `hostNetwork`, so the `traefik` namespace can run the
`restricted` Pod Security level, and a rolling update can start the surge pod
while the old pod still serves (`maxUnavailable: 0`, `maxSurge: 1`).

The base runs one replica. The baremetal overlay sets `deployment.replicas: 2`
so a drain or rollout never leaves the edge empty. The PodDisruptionBudget uses
`maxUnavailable: 1`: with one replica it never blocks a node drain, and with two
it keeps one pod serving. The pods run with `priorityClassName:
platform-critical` from the cluster-baseline base. Helm creates and replaces the
chart CRDs on install and upgrade (`crds: CreateReplace`).

The API/dashboard is disabled. Ping on the internal `traefik` entrypoint and
Prometheus metrics on internal port 9100 are not exposed by the Service.

Keep ServiceLB enabled on k3s (do not pass `--disable=servicelb`). A cluster
without ServiceLB needs another LoadBalancer implementation, for example the
Hetzner CCM with a managed Load Balancer.

Before applying this base, disable k3s's packaged Traefik on every server:

```yaml
# /etc/rancher/k3s/config.yaml
disable: [traefik]
```

Restart k3s after changing that file and remove any old packaged Traefik
resources before Flux reconciles this release.

The chart release ships `default-rate-limit` and `default-connection-limit`
Middleware objects and attaches them to both public entrypoints, so every route
gets the baseline limits. An overlay may patch the thresholds or replace the
entrypoint middleware list with a reviewed alternative. The objects are
rendered through the chart so Helm installs the Traefik CRDs before creating
them.

## Edge hardening

Every TLS router uses the `default` TLSOption in the `traefik` namespace,
which the chart renders from `tlsOptions.default`: TLS 1.2 is the minimum,
`sniStrict` rejects handshakes whose SNI matches no certificate, curves are
X25519, P-256 and P-384, and TLS 1.2 offers only ECDHE AES-GCM and
ChaCha20-Poly1305 suites. Go fixes the TLS 1.3 suites.

The `web` entrypoint answers every request with a permanent redirect to
`websecure` (`ports.web.http.redirections.entryPoint`). The `websecure`
entrypoint attaches `default-security-headers` after the limit middlewares. It
sets HSTS for one year with `includeSubDomains`, `X-Content-Type-Options:
nosniff`, `X-Frame-Options: DENY` and `Referrer-Policy:
strict-origin-when-cross-origin`. A route that must be framed or must not
preload HSTS needs its own reviewed entrypoint, not a weaker default.

Access logs are JSON, drop every request header and drop the query string from
`RequestPath` (`accessLog.fields.queryParameters.defaultMode: drop`, Traefik
3.7.3 or newer), so OAuth codes and tokens in URLs never reach Loki.
`global.checkNewVersion` and `global.sendAnonymousUsage` are off.

### Client IP overlay contract

`default-rate-limit` and `default-connection-limit` key on the client IP with
`sourceCriterion.ipStrategy.depth: 0`, which means the TCP peer address. On
k3s, ServiceLB forwards the connection without a proxy hop, so the peer is the
client and the base is correct as it is.

An overlay that puts Traefik behind a load balancer or proxy that terminates
the connection must set both of these, never just one:

- `ports.web.forwardedHeaders.trustedIPs` and
  `ports.websecure.forwardedHeaders.trustedIPs` to the proxy's source CIDRs, so
  Traefik keeps `X-Forwarded-For` only from those peers;
- `sourceCriterion.ipStrategy.depth` on both limit middlewares to the number of
  trusted hops, usually `1`, so the limit keys on the address the proxy saw.

With only one of the two, every client shares the proxy's bucket: Traefik
replaces an `X-Forwarded-For` from an untrusted peer with the peer address, and
depth `0` ignores the header.

`deploy/platform/e2e/tests/traefik-edge` checks the TLS versions, the redirect,
the headers and the access log on a live cluster.

## Local default certificate

`overlays/local/components/traefik` issues `local-default-tls` from the
`local-self-signed` ClusterIssuer for `localhost` and `*.localhost`, and adds
it to the `default` TLSStore (chart value `tlsStore.default`) both as a store
certificate and as the default certificate. With `sniStrict`, Traefik serves
only certificates whose SAN matches the SNI, and it never falls back to the
default certificate, so the store certificate entry is what makes routes
without their own secret work. The `local-traefik` Flux Kustomization depends
on `local-issuer` for that reason.

Local hosts must be exactly one label under `localhost`, such as
`fixture.localhost` or `api-fixture.localhost`. A wildcard covers one label
only: `localhost` itself is in the SAN list, but `api.fixture.localhost` is
not, and Traefik refuses its handshake. A product that needs deeper names
brings its own Certificate. The certificate is self-signed, so browsers and
clients must trust it explicitly or skip verification.

The base and other overlays get no default certificate: every host there
needs a TLS secret of its own.

## Monitoring

The chart emits a ServiceMonitor. The observability stack must install the
Prometheus Operator CRDs before the HelmRelease reconciles (typically by a Flux
`dependsOn` in the consuming overlay). The API check is disabled only so Helm
can render before that CRD exists; Kubernetes still requires the CRD at apply
time. The overlay supplies Ingresses, domains, TLS references, trusted proxy
CIDRs if a proxy is later introduced, and the replica count.

No dashboard route, ops route, Certificate, domain, credential, or Secret is
created here.

## Timeouts and retries

The `web` and `websecure` entrypoints close a client that takes more than 60 s
to send its request (`readTimeout`) or to read the response (`writeTimeout`),
and drop idle keep-alive connections after 180 s (`idleTimeout`). The default
ServersTransport gives a backend 5 s to accept the connection (`dialTimeout`)
and 35 s to send response headers (`responseHeaderTimeout`), 5 s above the app
request timeout so the app answers its own timeout first. Chart `42.4.1` maps
its `serversTransport` value only to the v3.8 read and write timeouts, so these
two go in through `additionalArguments`.

Retries belong to the app, not the entrypoint. The `baukit-app` chart and the
generated IngressRoute attach a `retry` Middleware (2 attempts, 100ms initial
interval) to the API route only. Traefik `v3.7.19` retries only when it cannot
reach a pod, stops once a pod answers whatever the status, and never retries
`POST`, `PATCH` or `LOCK`. There is no circuit-breaker Middleware: with one or
two API pods it would turn a partial failure into a full outage.
