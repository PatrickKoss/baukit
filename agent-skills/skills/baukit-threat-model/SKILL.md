---
name: baukit-threat-model
description: Build or update a Baukit product's `docs/threat-model.md` with data classes, trust boundaries, STRIDE per boundary, and a threat table that maps each threat to a Baukit control and a test. Use when starting a new product, adding a route group, an integration or webhook, a new data class, changing authentication or token handling, or before go-live.
---

# Threat-model a Baukit product

The output is `docs/threat-model.md`. Every row names a control and a test that proves it. A row without a test is a gap, and gaps stay visible until a task closes them.

Read `baukit.toml`, `CLAUDE.md`, `backend/`, `web/`, `mobile/`, the OpenAPI document, and the existing `docs/threat-model.md` first. Update the existing file in place. Keep row IDs stable so tasks and commits can cite them.

## 1. Inventory data classes

List each class once, with where it lives. Check every store, including the ones nobody thinks of:

- server PostgreSQL tables (owner column, retention, erasure path through `baukit-erasure` / `baukit-data-subject`);
- device storage: Expo SQLite (`@baukit/data-contracts-expo-sqlite`), IndexedDB (`@baukit/data-contracts-dexie`), secure store for tokens;
- browser storage (`localStorage`, `sessionStorage`); tokens never belong here;
- object storage and media served through njs media grants;
- logs, traces and metrics (`baukit-telemetry`): what could leak into fields and labels;
- analytics (`@baukit/analytics-core`, PostHog adapters): which properties are sent;
- third parties reached through `baukit-egress`, and secrets in `baukit-credential-vault`.

Mark each class: personal, credential, financial, health, content, operational.

## 2. Draw trust boundaries

Use the ones the product actually has:

| Boundary | Untrusted side sends |
|---|---|
| browser to API | bearer token, JSON bodies, headers, CORS origin |
| native app to API | bearer token, sync batches, push tokens |
| API to worker / jobs | job payloads written by the API |
| MCP client to MCP endpoint | JSON-RPC, bearer or API token, Origin |
| client to njs media edge | grant query string |
| IdP to API | JWKS, ID and access tokens (OIDC, Clerk, WorkOS) |
| API to third-party API via egress | URLs, responses, redirects |
| inbound webhook to API | signed body, signature headers |
| suite hub to product | signed events, tickets, plan and organization data |

## 3. STRIDE per boundary

For each boundary, ask all six questions and write a row for every real threat:

- Spoofing: can someone act as another user, client, IdP or peer?
- Tampering: can bytes change in flight or at rest without detection?
- Repudiation: is there an audit trail or receipt for the action?
- Information disclosure: cross-tenant reads, existence leaks, secrets in logs, verbose errors?
- Denial of service: unbounded bodies, rows, fan-out, retries, expensive queries?
- Elevation of privilege: scope or role bypass, MCP tools beyond the grant, admin routes?

## 4. Map each threat to a control and a test

Name the Baukit control with its crate or package (`baukit-auth` verifier profile, `baukit-http` body limit, `baukit-egress` `GuardedClient`, `baukit-ratelimit`, `baukit-core` webhook signatures and media grants, `baukit-mcp` origin and bearer checks, `baukit-credential-vault`, `baukit-sync` owner revisions). If nothing covers it, write `none` and set status `gap`.

Each row needs a test path. Use an existing test when it exercises the product's configuration. Otherwise name the test to write and add it in the same change, or set status `gap` with the task ID. The `baukit-security-testing` skill has the skeletons.

Status values: `covered` (control and passing test), `untested` (control exists, test missing), `gap` (no control), `accepted` (risk accepted, with the reason and who decided).

## 5. Write `docs/threat-model.md`

```markdown
# Threat model

Last reviewed: YYYY-MM-DD, commit <sha>.

## Data classes

| Class | Kind | Stores | Retention and erasure |
|---|---|---|---|

## Trust boundaries

| Boundary | Entry points | Identity |
|---|---|---|

## Threats

| ID | Boundary | STRIDE | Threat | Control | Test | Status |
|---|---|---|---|---|---|---|
| T-01 | browser to API | I | User A reads user B's note by ID | queries scoped by `(owner_id, id)`, missing row maps to `ApiError::not_found` (`baukit-http`) | `backend/tests/notes_cross_tenant.rs::other_users_note_is_not_found` | covered |

## Re-run when

- a route group, integration, webhook, MCP tool or worker job is added;
- a data class or store is added, including analytics properties;
- authentication, token storage or IdP configuration changes;
- before go-live and before enabling a new deployment target.
```

## 6. Finish

- Every threat row has a control or `gap`, and a test path or a task ID.
- Every `covered` test runs in the product quality gate. Run it.
- New gaps go to the product's task list with the row ID.
