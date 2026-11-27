# {{ context.app_name }} threat model

The template generated this file from the capabilities in `baukit.toml`. Each pre-filled row names a boundary the generated code has and the test or check that covers it. Replace every `TODO(owner)` before the first release, and add a row when you add a route, webhook, integration, queue or file upload.

## Data classes

| Class | Kind | Stores | Retention and erasure |
|---|---|---|---|
{% if context.backend %}| Items | product content | PostgreSQL `items` (`backend/migrations/0001_create_items.sql`), every query filtered by `owner_id` | TODO(owner) |
{% endif %}{% if context.backend and context.auth_enabled %}| Identity links (provider subject to internal user) | personal | PostgreSQL `user_identities` (`backend/migrations/0002_create_user_identities.sql`) | `DELETE /me` erasure, `backend/tests/identity_erasure_conformance.rs` |
| Web browser sessions | credential | PostgreSQL `baukit_auth_browser_sessions`, refresh token sealed server side by the `/auth/session` backend-for-frontend (`backend/crates/{{ context.app_name }}-bin/src/browser_session.rs`) | 30 day idle and 90 day absolute timeout by default; erased with the profile (`backend/tests/browser_session_erasure.rs`) |
{% endif %}{% if context.mobile and context.auth_enabled %}| Native session tokens | credential | device secure store through `mobile/src/auth-storage.ts` | token lifetime; cleared on sign-out |
{% endif %}{% if context.platform %}| Platform entitlements and inbound receipts | operational | PostgreSQL `platform_entitlement_cache` and `platform_inbound_receipts` (`{{ context.platform_migration }}`) | receipts older than 30 days purged, `backend/tests/platform_integration.rs::signed_fixture_event_reads_entitlements_and_retains_receipts` |
{% endif %}{% if context.web or context.mobile %}| Local device data | product content | device storage scoped to the signed-in subject | TODO(owner) |
{% endif %}| Telemetry and logs | operational | TODO(owner) | TODO(owner) |
| Product data classes | TODO(owner) | TODO(owner) | TODO(owner) |

## Trust boundaries

| Boundary | Entry points | Identity |
|---|---|---|
{% if context.backend %}| client to API | product routes in `backend/crates/{{ context.app_name }}-api` | {% if context.auth_enabled %}bearer token from the {{ context.auth_provider }} issuer{% else %}none; TODO(owner) before the API stores user data{% endif %} |
| operator to ops port | health, readiness and metrics routes on the operations listener{% if context.worker %}, `POST /admin/jobs/requeue` on the same listener{% endif %} | {% if context.worker %}admin bearer token for requeue, none for probes{% else %}none; cluster network only{% endif %} |
| API to PostgreSQL | `backend/crates/{{ context.app_name }}-postgres` | database role from configuration |
{% endif %}{% if context.backend and context.auth_enabled %}| IdP to API | discovery and JWKS fetch | issuer URL |
| browser to session backend-for-frontend | `/auth/session` routes | sealed session cookie |
{% endif %}{% if context.web %}| browser to web app | static bundle and its Content Security Policy | none |
{% endif %}{% if context.mobile %}| device to native app | deep links and the local store | device owner |
{% endif %}{% if context.mcp %}| MCP client to MCP endpoint | `/mcp` and `/.well-known/oauth-protected-resource/mcp` | bearer token for the MCP resource audience |
{% endif %}{% if context.worker %}| API to worker queue | `job_outbox` rows consumed by the worker | database role |
{% endif %}{% if context.platform %}| platform hub to API | `POST /platform/inbound` | HMAC signature with the receiver secret |
| API to platform hub | entitlement ticket pull | service token out, ticket signed with the hub JWKS in |
{% endif %}| API to third parties | TODO(owner) | TODO(owner) |

## Threats

| ID | Boundary | STRIDE | Threat | Control | Test | Status |
|---|---|---|---|---|---|---|
{% if context.backend %}| B-01 | client to API | I, T | A second owner reads or changes another owner's item | repository queries put `owner_id` in the WHERE clause | `backend/tests/conformance.rs::in_memory_items_are_owner_scoped`, `backend/tests/postgres_integration.rs::postgres_adapter_crud` | covered |
| B-02 | client to API | D | Request floods and oversized bodies | rate limiter, body limits and security headers on every served route | `backend/tests/composed/layer_coverage.rs::every_route_is_limited_capped_and_hardened`, `backend/tests/composed/limiter_socket.rs::exhausting_one_client_ip_leaves_another_unthrottled`, `baukit check security` | covered |
| B-03 | client to API | I | Error bodies echo input or internal causes | `ApiError` envelope with stable reason codes | `backend/tests/conformance.rs::json_rejections_keep_their_protocol_statuses`, `backend/tests/composed/router_proptest.rs::random_requests_fail_closed_without_server_errors` | covered |
| B-04 | operator to ops port | I, E | Probes and metrics reachable from outside the cluster | the `baukit-app` chart schema accepts only `ClusterIP` for `opsService.type`, and ops routes live on a separate listener | `baukit check security`, `backend/tests/conformance.rs::health_and_metrics_conform_to_baukit` | covered |
{% endif %}{% if context.backend and context.auth_enabled %}| A-01 | client to API | S | Item routes accept a second principal, no token or a forged token | per-route auth, cross-tenant and negative-token probes over every OpenAPI operation | `backend/tests/conformance.rs::item_routes_deny_other_tenants_and_invalid_tokens`, `backend/tests/auth_conformance.rs::protected_route_conforms_and_maps_subject_to_internal_user` | covered |
| A-02 | IdP to API | S | Token from another issuer or authorized party | issuer and authorized-party checks in the verifier | `backend/tests/provider_config.rs::external_issuer_configuration_selects_the_verifier_end_to_end`, `backend/tests/provider_config.rs::oidc_authorized_parties_restrict_rest_sessions_only_when_configured` | covered |
| A-03 | client to API | D | Bad bearers burn verifier work | authentication runs before identity rate limiting, bad bearers share the IP allowance | `backend/tests/auth_conformance.rs::authentication_runs_before_identity_rate_limiting`, `backend/tests/composed/auth_order.rs::bad_bearers_are_verified_at_most_once_per_ip_allowance` | covered |
| A-04 | client to API | I | Erased user data or old sessions survive `DELETE /me` | identity erasure with fences, browser sessions deleted for the erased subject only | `backend/tests/identity_erasure_conformance.rs::endpoint_identity_erasure_conforms`, `backend/tests/identity_erasure_conformance.rs::retained_identity_reonboards_with_a_new_profile_and_rejects_old_sessions`, `backend/tests/browser_session_erasure.rs::profile_erasure_deletes_only_the_erased_subjects_browser_sessions` | covered |
| A-05 | IdP to API | S | Production starts with placeholder identity secrets or plain HTTP | configuration validation outside local | `backend/tests/identity_erasure_conformance.rs::identity_configuration_requires_production_secrets_and_https` | covered |
| A-06 | browser to session backend-for-frontend | D | Session endpoint floods | per-IP and per-session buckets | `backend/crates/{{ context.app_name }}-bin/src/browser_session.rs::rate_limits_default_to_the_adr_values` | covered |
{% endif %}{% if context.web %}| W-01 | browser to web app | T | Injected inline script runs | Content Security Policy hashes every inline script, the build check fails otherwise | `web/csp/check-build.test.ts` | covered |
| W-02 | browser to web app | I | Token written to `localStorage`, `sessionStorage` or IndexedDB | `ts-token-web-storage` ast-grep rule{% if context.backend and context.auth_enabled %}; the web client keeps the refresh token behind the session backend-for-frontend{% endif %} | `baukit check patterns` | covered |
{% endif %}{% if context.web and context.auth_enabled %}| W-03 | browser to web app | I | A second user on the same browser sees the first user's cached data | cache closes and resets before another subject becomes ready | `web/src/persistence-lifecycle.test.ts` | covered |
{% endif %}{% if context.web and context.auth_oidc %}| W-04 | IdP to API | S | Authorization code intercepted | authorization code flow with S256 PKCE | `web/src/auth.test.ts` | covered |
{% endif %}{% if context.mobile %}| M-01 | device to native app | I | Local data read by the next user of the device | TODO(owner) | TODO(owner) | open |
{% endif %}{% if context.mobile and context.auth_enabled %}| M-02 | device to native app | I | Session tokens stored outside the secure store | `mobile/src/auth-storage.ts` refuses web storage | `mobile/src/auth-storage.test.ts`{% if context.auth_oidc %}, `mobile/src/oidc-auth.test.tsx`{% endif %} | covered |
| M-03 | device to native app | I | A new subject opens the previous subject's store | persistence closes, resets and opens per subject | `mobile/src/persistence-lifecycle.test.ts` | covered |
{% endif %}{% if context.mcp %}| C-01 | MCP client to MCP endpoint | S, E | A REST token or a token for another client reaches MCP tools | separate MCP verifier bound to the `/mcp` resource audience and client | `backend/tests/provider_config.rs::mcp_enforces_provider_token_binding_and_introspection_is_restricted` | covered |
| C-02 | MCP client to MCP endpoint | E | Tool schemas or required scopes change unreviewed | committed `backend/mcp-tools.json` | `backend/tests/tool_drift.rs::registered_tools_match_committed_schema` | covered |
| C-03 | MCP client to MCP endpoint | S | Cross-site page drives `/mcp` | TODO(owner) | TODO(owner) | open |
{% endif %}{% if context.worker %}| J-01 | operator to ops port | S, E | Requeue reachable without the admin token or from the public API | requeue mounted only with a configured token, rate limited, absent from the API router | `backend/tests/requeue_mount.rs::the_requeue_route_rejects_a_wrong_token`, `backend/tests/requeue_mount.rs::the_requeue_route_is_absent_without_an_admin_token`, `backend/tests/requeue_mount.rs::the_requeue_route_answers_429_once_the_ip_quota_is_used_up`, `backend/tests/conformance.rs::health_and_metrics_conform_to_baukit` | covered |
| J-02 | API to worker queue | T, D | A job is lost or runs twice during a database pause | durable outbox | `backend/tests/worker_integration.rs::a_postgres_pause_mid_job_neither_loses_nor_doubles_the_job` | covered |
{% endif %}{% if context.platform %}| P-01 | platform hub to API | S, R | Forged or replayed hub delivery | HMAC signature with the receiver secret, receipts deduplicate deliveries | `backend/tests/platform_integration.rs::signed_fixture_event_reads_entitlements_and_retains_receipts` | covered |
| P-02 | API to platform hub | S | Entitlement ticket for another subject accepted | ticket verified against the hub JWKS and bound to the subject | `backend/tests/platform_integration.rs::ticket_source_rejects_a_signed_ticket_for_another_subject` | covered |
| P-03 | platform hub to API | D | `/platform/*` routes skip the rate limiter | platform routes merged inside the limiter | `backend/tests/platform_integration.rs::platform_routes_sit_inside_the_rate_limiter`, `baukit check security` | covered |
{% endif %}| T-01 | API to third parties | S, I | TODO(owner) | outbound HTTP through `baukit-egress` | TODO(owner) | open |
