# {{ context.app_name }}

{{ context.product_description }} generated with Baukit template {{ context.template_version }}.

## Run locally

The API uses the in-memory repository when `{{ context.app_env }}__DATABASE__URL` is absent. To use PostgreSQL, configure the standard database section and run migrations explicitly:

```sh
export {{ context.app_env }}__DATABASE__URL=postgres://postgres:postgres@localhost{% if context.port_offset > 0 %}:{{ context.postgres_host_port }}{% endif %}/{{ context.app_crate }}
make migrate
make run
```

Migrations are never run during API startup. The public API listens on port {{ context.api_host_port }} and private health, readiness, metrics, and build endpoints listen on port {{ context.ops_host_port }} by default.

`backend/Dockerfile` has separate `api`, `migrate`{% if context.worker %}, and
`worker`{% endif %} runtime targets. Build each process from the backend context,
for example `docker build --target api -t {{ context.app_name }}-api:local backend`.
Pass `--build-arg GIT_COMMIT=$(git rev-parse --short=12 HEAD)` to record the
source revision in `build_info`; omitted build args retain the `unknown` default.
For a checkout generated with `--baukit-path`, use the Baukit repository root as
the context and pass `BACKEND_CONTEXT`, `BAUKIT_CONTEXT`, and the generated
absolute Cargo path as `BAUKIT_DESTINATION`; this keeps local path dependencies
inside the Docker build context without editing generated source.
{% if context.worker %}
`make run-worker` starts the durable worker, which requires PostgreSQL and exposes only the private operations listener. Its `[worker]` product configuration is available through `{{ context.app_env }}__WORKER__CONCURRENCY`, `{{ context.app_env }}__WORKER__LEASE_DURATION_SECONDS`, `{{ context.app_env }}__WORKER__JOB_TIMEOUT_SECONDS`, and `{{ context.app_env }}__WORKER__POLL_INTERVAL_MILLISECONDS`; the generated deploy values carry the same defaults. Creating an item through the PostgreSQL adapter atomically emits the demo `item.created` outbox job. The generated handler logs identifiers only, and the ignored Docker integration test proves the real claim, handle, and completion path.
{% endif %}
{% if context.auth_oidc %}
Every generated API route requires a bearer token. `GET /me` also maps the token's stable `sub` claim to an internal user UUID. Auth configuration follows the product convention `{{ context.app_env }}__AUTH__ISSUER` and `{{ context.app_env }}__AUTH__AUDIENCE`, defaulting to the composed realm and `{{ context.app_name }}-backend` audience.

`docker compose up -d --wait postgres keycloak` starts PostgreSQL and waits for Keycloak's readiness endpoint. Sign in as `test` / `password`; the imported realm contains the confidential backend client{% if context.web %}, a PKCE-only web client{% endif %}{% if context.mobile %}, and a PKCE-only mobile client{% endif %}. The checked-in credentials are development-only.

Keycloak's development hostname is deliberately dynamic. Discovery through `http://localhost:{{ context.keycloak_host_port }}/realms/{{ context.app_name }}` advertises `localhost`; discovery through `http://127.0.0.1:{{ context.keycloak_host_port }}/realms/{{ context.app_name }}` advertises `127.0.0.1`. Pick one spelling and use it consistently for backend issuer configuration, browser/mobile configuration, discovery, and token validation. Prefer `localhost` for browser development: Keycloak may mark its login cookie `Secure`, and browsers give `localhost` special secure-context treatment that is not portable to arbitrary HTTP hostnames. The generated headless helper accepts that cookie over local HTTP solely for disposable development; production issuers must use HTTPS.

{% if context.web or context.mobile %}After the API is running, exercise discovery, PKCE, and authenticated `/me` without a client secret:

```sh
python3 scripts/pkce-login.py \
  --issuer http://localhost:{{ context.keycloak_host_port }}/realms/{{ context.app_name }} \
  --client-id {{ context.app_name }}-{% if context.web %}web{% elif context.mobile %}mobile{% else %}backend{% endif %}
```

{% if context.mobile %}For the mobile client, also pass `--redirect-uri {{ context.app_name }}://oauth`. {% endif %}The helper's client ID is always explicit so product smoke tests cannot silently use another product's client.
{% else %}This realm has no public PKCE client. Add a product-owned public client before using `scripts/pkce-login.py`; its `--client-id` argument is mandatory so smoke tests cannot silently use another product's client.
{% endif %}
{% endif %}

Useful commands:

```sh
make preflight
make check
make openapi
baukit doctor
make openapi-client
```

`make preflight` fails before dependency resolution when the generated product
needs a private Git dependency but its SSH agent is missing, unusable, or has no
loaded identity. Set `BAUKIT_PREBUILT_IMAGES=true` only when the required images
already exist and no build will fetch private dependencies. If the web product
adds Playwright, the same script checks, installs, and runs a supplied command
with browsers under the repository-local
`web/node_modules/.cache/playwright-browsers` cache (for example,
`sh scripts/preflight.sh -- corepack pnpm --dir web exec playwright test`).

`.github/workflows/ci.yml` runs every generated backend{% if context.web %}, web{% endif %}{% if context.mobile %}, and mobile{% endif %} gate, including ignored Docker-backed Rust tests. `deploy/values.yaml` is the product-owned input for the shared `baukit-app` Helm chart. Matching backend workflow notes are installed for both Codex and Claude discovery paths.

`make openapi` refreshes the committed backend schema. `make openapi-client` consumes that schema without rebuilding the backend or requiring `baukit` on `PATH`; it uses current Node.js LTS with corepack or npx and writes `generated/openapi.d.ts`.

## Backend layout

- `{{ context.app_name }}-domain`: business types and invariants; no framework dependencies.
- `{{ context.app_name }}-ports`: repository traits and boundary errors.
- `{{ context.app_name }}-services`: use cases that depend only on ports.
- `{{ context.app_name }}-api`: Axum routes, DTOs, error mapping, and Utoipa schema.
- `{{ context.app_name }}-postgres`: one SQLx adapter per aggregate (`PostgresItemRepository`, `PostgresUserRepository`, and future peers), all allowed to share a pool without growing one catch-all repository.
{% if context.worker %}- `{{ context.app_name }}-worker`: static job contracts and handlers executed by `baukit-jobs::WorkerRunner`; the PostgreSQL item transaction writes the durable outbox row.
{% endif %}
- `{{ context.app_name }}-bin`: API composition plus `migrate` and `openapi` binaries; its API composition includes the in-memory adapter.
- `backend/tests`: Baukit conformance, OpenAPI drift, and ignored Docker-backed PostgreSQL tests.

The workspace consumes Baukit from {{ context.baukit_dependency_description }}. Generated applications build and run directly with Cargo and do not need the Baukit CLI.

By default, `baukit new` resolves and emits Cargo and pnpm lockfiles at scaffold time. Keep them committed, update them with `make lockfiles` after dependency changes, and use `--locked` / `--frozen-lockfile` in automation. Offline generation can use `--skip-lockfiles`, but `sh scripts/lockfiles.sh` must run before the first build.

## Repository setup

Generation never commits or pushes on your behalf. For a new directory:

```sh
git init
git add .
git commit -m "Scaffold {{ context.app_name }} with Baukit"
git remote add origin git@github.com:YOUR_ORG/{{ context.app_name }}.git
git push -u origin main
```

For an existing or orphan-branch repository root, run `baukit new {{ context.app_name }} ... --dir . --into-existing`; existing differing files are reported as conflicts and never overwritten.
