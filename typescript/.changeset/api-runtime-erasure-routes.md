---
'@baukit/api-runtime': minor
---

`createProfileErasureClient` accepts `routes: { eraseMethod, erasePath, statusPath }`, so a product that erases through another route, such as `POST /v1/me/erasure` with `GET /v1/me/erasure/{operationId}`, uses the client directly. `templateProfileErasureRoutes` holds the default `DELETE /me` and `GET /me/erasures/{operationId}`. A status path without `{operationId}` throws a `TypeError` when the client is created.
