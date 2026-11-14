# PostHog analytics base

This optional base installs the smallest PostHog stack that Baukit currently
supports for low-volume, local product analytics. It is inert unless a cluster
overlay explicitly composes `deploy/platform/posthog/`; no other Baukit base or
chart references it.

The application renderer is the final published PostHog Kubernetes chart,
`30.46.0` (PostHog `1.43.0`). PostHog ended Kubernetes support in 2023, so the
chart is treated as a pinned renderer rather than an upgrade stream. The
PostHog, ClickHouse 22.8.21.38, PostgreSQL 14.25, and Redis 6.2.24,
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
flag service, Temporal, email, backups, toolbox, the bundled object store, and the
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

The chart uses external S3 storage at
`http://rustfs.postgres.svc.cluster.local:9000`, bucket `posthog`, region
`us-east-1`. The local foundation creates the bucket. The pinned chart disables
its bundled object store by default. PostHog 1.43.0's plugin client forces
path-style addressing, and its Python client uses the custom endpoint with
Boto's automatic path-style selection. Session recording remains disabled.

PostHog 1.43.0 refuses PostgreSQL above 14.1.0. 14.25 only adds patch releases
on the same major, so the release sets `SKIP_SERVICE_VERSION_REQUIREMENTS=1`.
ClickHouse and Redis sit inside its ranges.

## Network fence

The namespace starts from the shared `default-deny` and `allow-dns-egress`
components. `network-policy.yaml` then opens only these pairs:

- Traefik to `posthog-web` and `posthog-events` on 8000;
- PostHog app pods to PostgreSQL 5432, Redis 6379, Redpanda 9092, ClickHouse
  8123 and 9000, and RustFS in namespace `postgres` on 9000;
- ClickHouse to Redpanda 9092 and ZooKeeper 2181;
- the plugin server to public addresses on 443, for plugin HTTP calls.

Everything else in or out of the namespace is denied, including traffic
between the dependencies that the pairs above do not name.

## Pod Security gaps

Every dependency in `dependencies.yaml` meets `restricted`. The chart
workloads do not, and the pinned chart has no values to fix them. These
containers set no `runAsNonRoot`, `allowPrivilegeEscalation: false`,
`capabilities.drop: [ALL]` or seccomp profile. They pass `baseline`, since nothing
runs privileged or on host namespaces, but fail `restricted`:

- `posthog-web`: `wait-for-service-dependencies`, `wait-for-migrations`, `posthog-web`;
- `posthog-events`: `wait-for-service-dependencies`, `posthog-events`;
- `posthog-plugins`: `wait-for-service-dependencies`, `wait-for-migrations`, `posthog-plugins`;
- migrate Job: `wait-for-service-dependencies`, `migrate-job`;
- Grafana annotation hook Job: `grafana-annotation-job-post`.

Closing them needs post-render patches on the HelmRelease; until then the
namespace carries no `restricted` enforce label.

## Overlay contract

Before composition, create `Secret/posthog-secrets` in namespace `posthog`
through the consuming cluster's SOPS flow with these keys:

- `posthog-secret`: at least 50 random bytes, used by Django;
- `postgresql-password`: a generated database password;
- `clickhouse-password`: a generated ClickHouse password.

Also create `Secret/posthog-object-storage` in `posthog` with `root-user` and
`root-password` keys for RustFS. The local integration helper copies its
RustFS credentials into that Secret. A consuming overlay must compose the
local foundation or replace `spec.values.externalObjectStorage` with its own
endpoint, bucket, region, and credential Secret.

Patch `spec.values.siteUrl`, `web.secureCookies`, and the chart ingress values
for the instance hostname/TLS policy. The checked-in `posthog.invalid` URL and
disabled ingress are safe base defaults, not an operable public endpoint.

The resource settings are the measured low-traffic local envelope documented
by the consuming overlay. Re-measure before using this base on another node or
enabling excluded PostHog products. PostgreSQL, ClickHouse, Redpanda, and
ZooKeeper PVCs are retained if the Flux Kustomization is removed; delete those
claims only after following the consumer's disable/backup runbook.
