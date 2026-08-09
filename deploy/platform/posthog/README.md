# PostHog analytics base

This optional base installs the smallest PostHog stack that Baukit currently
supports for low-volume, local product analytics. It is inert unless a cluster
overlay explicitly composes `deploy/platform/posthog/`; no other Baukit base or
chart references it.

The application renderer is the final published PostHog Kubernetes chart,
`30.46.0` (PostHog `1.43.0`). PostHog ended Kubernetes support in 2023, so the
chart is treated as a pinned renderer rather than an upgrade stream. The
PostHog, ClickHouse 22.8.21.38, PostgreSQL 14.1, and Redis 6.2.6 (each inside
PostHog 1.43.0's compatibility ranges),
Redpanda, ZooKeeper, and BusyBox images
are pinned by both version and digest. The chart's
bundled PostgreSQL, Redis, Kafka, ZooKeeper, ingress controller, cert-manager,
and observability components are disabled; small single-node dependencies live
in this base instead. A zero-pod `posthog-pgbouncer` Service alias maps the
legacy chart's mandatory port 6543 check directly to PostgreSQL; no PgBouncer
process is installed. Redpanda provides the Kafka API without a second Kafka
JVM, while ZooKeeper remains because PostHog's supported ClickHouse schema uses
replicated tables.

The ClickHouse image is fetched through Google's public Docker Hub mirror but
keeps the upstream manifest-list digest. This avoids anonymous Docker Hub rate
limits without substituting image contents.

The enabled product path is only `web` + `events` + combined `plugins`
ingestion, backed by PostgreSQL, Redis, Redpanda, ZooKeeper, and one ClickHouse
replica. The combined plugin server is explicitly limited to one worker with
two concurrent tasks so it does not derive 24 workers from the local node.
Celery worker, session recording, recordings ingestion/API, feature
flag service, Temporal, object storage, email, backups, toolbox, MinIO, and the
chart's ClickHouse operator/exporter and Grafana/Loki/Prometheus stack are
excluded. Single-node ClickHouse is managed directly like the other dependencies,
avoiding operator/CRD lifecycle overhead. The exclusions do not participate in
capturing or querying typed product events and would materially increase the
idle footprint. This is intentionally a local proof deployment, not the
supported current PostHog hobby topology or a production recommendation.
GeoIP/MMDB enrichment is also disabled so startup does not depend on an external
database download; IP location properties are therefore unavailable.
Migration Jobs receive a five-minute completion TTL so failed retry pods do not
become permanent cluster debris after a successful retry. Their dependency
probe also has a bounded connection timeout so a transient PostgreSQL endpoint
gap during an upgrade cannot hang the hook.

## Overlay contract

Before composition, create `Secret/posthog-secrets` in namespace `posthog`
through the consuming cluster's SOPS flow with these keys:

- `posthog-secret`: at least 50 random bytes, used by Django;
- `postgresql-password`: a generated database password;
- `clickhouse-password`: a generated ClickHouse password.

Patch `spec.values.siteUrl`, `web.secureCookies`, and the chart ingress values
for the instance hostname/TLS policy. The checked-in `posthog.invalid` URL and
disabled ingress are safe base defaults, not an operable public endpoint.

The resource settings are the measured low-traffic local envelope documented
by the consuming overlay. Re-measure before using this base on another node or
enabling excluded PostHog products. PostgreSQL, ClickHouse, Redpanda, and
ZooKeeper PVCs are retained if the Flux Kustomization is removed; delete those
claims only after following the consumer's disable/backup runbook.
