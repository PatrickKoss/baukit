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

`routes` in the API crate builds the product routes and `finalize_api` applies the Baukit HTTP layers. The API binary adds {% if context.auth_oidc %}authentication and rate limiting{% else %}any request middleware{% endif %} between the two, so every public response, including {% if context.auth_oidc %}401 and 429 rejections{% else %}middleware rejections{% endif %}, carries CORS headers, a request ID, and `Cache-Control: private, no-store`. Browsers can read `Retry-After` and the `RateLimit-*` headers. A handler that sets its own `Cache-Control` keeps it. The operations listener does not use these layers.

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

`make dev` starts Keycloak and Redis, validates the declared development policy, and reconciles the retained Keycloak volume. Run `make db-up` separately when the API needs PostgreSQL. The API's rate limiter connects to Redis at startup in every environment and exits with `RateLimitStoreError` when it cannot, so run `make dev` before `make run`. Compose publishes Redis on `127.0.0.1:{{ context.redis_host_port }}`{% if context.port_offset > 0 %} and `make run` passes that address as `{{ context.app_env }}__RATE_LIMIT__REDIS_URL`{% else %}, which matches the `{{ context.app_env }}__RATE_LIMIT__REDIS_URL` default of `redis://127.0.0.1/`{% endif %}. Deployments set that variable to their own Redis. Sign in as `test` / `development-password`; the imported realm contains the confidential backend client{% if context.web %}, a PKCE-only web client{% endif %}{% if context.mobile %}, and a PKCE-only mobile client{% endif %}. The checked-in credentials are development-only. Existing generated realms used the shorter `password` credential. The reconciler leaves that existing password unchanged unless you request a reset.

The development realm selects the unbranded `baukit-accessible` login theme. Compose mounts `keycloak/themes` read-only at `/opt/keycloak/themes`. The theme inherits `keycloak.v2` and adds two scripts. It does not copy Keycloak FreeMarker templates. The first script adds required-field semantics, linked live errors, and deliberate error focus to the inherited login and registration forms. A second script reads the app's appearance from the OAuth `state`. When the state looks like `ap1.<d|l|s>[.<PRIMARY>.<SECONDARY>].<nonce>`, the page keeps the requested light or dark mode even if the operating system prefers the other one, and it sets `--baukit-auth-primary`, `--baukit-auth-secondary`, and `--baukit-auth-on-primary` on the root element. The base theme does not paint with those colors, but a product child's CSS can. `@baukit/auth-native` builds that state with `appearanceStateDecoration`. Any other state leaves the page on its defaults. Registration remains disabled by default. Set `registrationAllowed` to `true` in `keycloak/realm.json` when the product needs self-registration, then run `make dev` to reconcile the retained realm.

Products can select `baukit-accessible` directly or add a product theme under `keycloak/themes/PRODUCT/login` with `parent=baukit-accessible`. Put product CSS in `resources/css`, translated message overrides in `messages`, and list the CSS in the child `theme.properties`. A child that adds its own `scripts` property must also list `js/accessibility.js` and `js/theme-preferences.js`, which Keycloak resolves through the parent. The generated `baukit-accessible-test` child is a neutral fixture that proves CSS, message, script, and parent lookup. Do not use that fixture as a product theme.

Restart Keycloak after editing a mounted theme so cached resources and theme properties cannot hide a change. Production deployments must package the theme in the pinned Keycloak image as described by the Baukit Operator base. A ConfigMap or development bind mount is not the production mechanism.

The compatibility contract covers Keycloak `26.7.0` and `26.7.1`. It depends on `#kc-form-login`, `#kc-register-form`, standard control IDs, `input-error-{name}`, and the PatternFly 5 or 6 required-marker and form-group classes inherited from `keycloak.v2`. Before changing the Keycloak patch or minor version, inspect those contracts and rerun the browser matrix. Run the fake-DOM gate with `make keycloak-theme-test`.{% if context.web %} Run the pinned real-browser matrix with credentials for the disposable generated realm:

```sh
KEYCLOAK_ADMIN_USERNAME=admin \
KEYCLOAK_ADMIN_PASSWORD=admin \
KEYCLOAK_TEST_USERNAME=test \
KEYCLOAK_TEST_PASSWORD=development-password \
make keycloak-theme-browser-test
```

{% endif %}

For an existing generated product, regenerate or copy the `keycloak/themes` tree, add the read-only Compose mount, add `loginTheme` to `keycloak/realm.json` and `keycloak/reconcile.json`, then run `make dev`. A product with an existing copied theme can instead change its parent to `baukit-accessible`, keep only product CSS and messages, and delete copied upstream templates after this browser matrix passes.

`keycloak/realm-policy.json` declares the environment class and the accepted password, TLS, brute-force, PKCE, direct-grant, and redirect bounds. Run `make keycloak-policy` after editing the realm. `keycloak/reconcile.json` selects the realm fields, public clients, users, origins, and redirects that `make keycloak-reconcile` may repair. The reconciler merges active URLs and leaves unselected live fields intact. It creates a missing selected user with the checked-in development credential, but it does not reset an existing password unless you pass `--reset-password USERNAME` to `scripts/reconcile_keycloak.py`.

If the configured development administrator no longer authenticates, the reconciler creates a random temporary recovery administrator while Keycloak is stopped, repairs the configured administrator, and removes the temporary account. It also attempts that cleanup when reconciliation is interrupted or fails. The script does not print administrator passwords, user credentials, access tokens, or Keycloak response bodies.

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
make setup
make preflight
make check
{% if context.auth_oidc %}make keycloak-policy
make dev
{% endif %}make openapi
baukit doctor
make openapi-client
```

`make setup` creates `web/.env` and `mobile/.env` when those capabilities exist. Later runs append assignments that were added to the matching `.env.example`, in example-file order, without replacing local values, comments, whitespace, or line endings. The script prints added key names but never their values. Any existing assignment wins, including `export`, blank, and quoted assignments. For duplicate example keys, the first assignment is used. For duplicate local keys, the file is left unchanged.

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

Handlers document only the statuses they return themselves. `error_response_rules()` in `{{ context.app_name }}-api` documents the rest from the middleware: 400, 413, 415, and 422 on operations with a JSON body, 400 and 404 on operations with a path parameter,{% if context.auth_oidc %} 401 on secured operations, 429 on every operation,{% endif %} and 500 and 504 on every operation. It also adds `X-Request-Id` to every response{% if context.auth_oidc %}, `Retry-After` to 429, and `WWW-Authenticate` to 401{% endif %}. Add a rule there when middleware starts returning a new status, then run `make openapi` and `make openapi-client`.

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

The product-root `limits.json` contains example resource limits. The domain `limits` module embeds and validates it, and web and mobile load the same file. Read `docs/resource-budgets.md` before replacing the values.

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

Existing generated products can adopt append-only environment setup by copying `scripts/setup.sh`, `scripts/reconcile-env.py`, and its test from the current template, then replacing instructions that copy `.env.example` over `.env` with `make setup`. Existing `.env` bytes remain unchanged. The script only appends missing assignments.
