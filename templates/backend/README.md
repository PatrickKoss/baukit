# {{ context.app_name }}

Backend-only Baukit product generated with template {{ context.template_version }}.

## Run locally

The API uses the in-memory repository when `{{ context.app_env }}__DATABASE__URL` is absent. To use PostgreSQL, configure the standard database section and run migrations explicitly:

```sh
export {{ context.app_env }}__DATABASE__URL=postgres://postgres:postgres@localhost/{{ context.app_crate }}
make migrate
make run
```

Migrations are never run during API startup. The public API listens on port 8080 and private health, readiness, metrics, and build endpoints listen on port 9090 by default.
{% if context.auth_oidc %}
The protected `GET /me` route verifies OIDC access tokens and maps their stable `sub` claim to an internal user UUID. Auth configuration follows the normal product convention: `{{ context.app_env }}__AUTH__ISSUER` and `{{ context.app_env }}__AUTH__AUDIENCE`, defaulting to the composed realm and `{{ context.app_name }}-backend` audience.

`docker compose up -d postgres keycloak` starts PostgreSQL plus the local Keycloak realm at `http://localhost:8081/realms/{{ context.app_name }}`. Sign in as `test` / `password`; the imported realm also contains the confidential backend client and PKCE-only public web/mobile clients. The checked-in credentials are development-only.
{% endif %}

Useful commands:

```sh
make check
make openapi
baukit doctor
baukit generate openapi-client
```

`docker compose up -d postgres{% if context.auth_oidc %} keycloak{% endif %}` starts the matching local dependencies. `.github/workflows/ci.yml` runs formatting, lint, tests, and the schema drift check, while `deploy/values.yaml` is the product-owned input for the shared `baukit-app` Helm chart. Matching backend workflow notes are installed for both Codex and Claude discovery paths.

TypeScript generation requires current Node.js LTS plus pnpm or npx. `openapi-typescript` is invoked with `pnpm dlx` (or `npx` when pnpm is unavailable) and writes `generated/openapi.d.ts`.

## Backend layout

- `{{ context.app_name }}-domain`: business types and invariants; no framework dependencies.
- `{{ context.app_name }}-ports`: repository traits and boundary errors.
- `{{ context.app_name }}-services`: use cases that depend only on ports.
- `{{ context.app_name }}-api`: Axum routes, DTOs, error mapping, and Utoipa schema.
- `{{ context.app_name }}-postgres`: SQLx repository adapter.
- `{{ context.app_name }}-bin`: API composition plus `migrate` and `openapi` binaries; its API composition includes the in-memory adapter.
- `backend/tests`: Baukit conformance, OpenAPI drift, and ignored Docker-backed PostgreSQL tests.

The workspace consumes Baukit from {{ context.baukit_dependency_description }}. Release-generated products use the pinned `v{{ context.template_version }}` git tag. For local development and fixture CI, regenerate with `--baukit-path /path/to/baukit/rust` to use path dependencies instead. Generated applications build and run directly with Cargo and do not need the Baukit CLI.
