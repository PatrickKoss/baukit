---
name: baukit-backend
description: Work safely in the generated {{ context.app_name }} Baukit backend.
---

# Baukit backend

Keep domain logic in `backend/crates/{{ context.app_name }}-domain`, boundary traits in ports, use cases in services, and concrete integrations in adapters. HTTP DTOs and Utoipa annotations belong in the API crate; process wiring belongs in the bin crate.

Run `make check` after changes. Run `make openapi` and commit `backend/openapi.json` when the API contract changes. Never run migrations from API startup; use `make migrate`.
