# W1-I: tenancy scoping and rate limiting across 12 products

This is a read-only audit. I wrote no repo files. Per-product sections follow, and every path:line in them still needs orchestrator verification. Five products got the full audit: solo-leveling-system, kassensturz, lesezeichen, eigenruhe and schluesselbund. The other seven got steps 1, 3 and 5 in full, with steps 2 and 4 sampled.

## Cross-product summary

### Confirmed missing scoping (full call path read)
- kassensturz, lesezeichen and schluesselbund: `GET /items`, `GET/PUT/DELETE /items/{id}` is template CRUD on a global `items` table with no owner column. Any signed-in user can read, edit or delete every row. Fix this in templates/ as well.
- solo-leveling-system: any authenticated user can read private guilds (`/guilds/{id}`, members, progression, quests, quest-templates, competitions) and `/parties/{id}` by id. Adding a cohort member enrolls any user id in the quest arc before the FK rejects the row (services/src/orgs.rs:1934-1956). The OAuth callback trusts the signed state for the user, which is a possible CSRF. I did not check whether the state is single-use.
- redemut: sync push does not check a pushed event's device_id against the caller (services/src/lib.rs:1299), so another user's device can be referenced.

### Service-check only (latent)
- baukit-suite `find_link_for_owner` (baukit-suite/src/adapters/postgres/mod.rs:539-548) loads by id and filters in Rust. `refresh_failures` writes before that check, and in kassensturz's case the revoke UPDATE also filters on id only. Every product that uses suite inherits this, so it is a baukit fix.
- kassensturz sync push: 9 tables use `ON CONFLICT(id) DO UPDATE` without a user_id guard and rely on an unlocked EXISTS pre-check, so there is a race window. hebkit has the same pattern for meal_template_entries and recipe_ingredients.
- solo-leveling-system: org teams, cohorts, metrics, guild and party invites, connections, reactions and categories. Org definitions and assignments are written with `WHERE id=$1`.
- schluesselbund: families (Rust organizer check) and org data queries behind a central authorize().
- tiefgang: 5 handlers. schlauzug: runs and room host view. leitbild: reflection-jobs owner taken from the JSON payload, plus the catalog import upsert can rewrite owner.

### SQL-scoped (good)
eigenruhe is SQL-scoped on all 64 id routes. lesezeichen is SQL-scoped on 29 of 32. architecture-health-platform uses require_member plus scoped SQL. runtime-analyzer uses Postgres FORCE RLS, but 39 direct pool calls and 62 SECURITY DEFINER uses bypass it. Most `/me` routes in kassensturz and solo-leveling-system are SQL-scoped too.

### Cross-tenant tests (routes with an id / routes with a two-principal test)
| Product | Routes with id | Routes with a two-principal test |
|---|---|---|
| eigenruhe | 64 | 42 |
| lesezeichen | 32 | 23 |
| schluesselbund | 49 | 26 |
| kassensturz | 29 | 6 |
| solo-leveling-system | about 175 | about 20 |
| leitbild | 18 | 4 |
| tiefgang | 18 | about 2 |
| redemut | 7 sampled | 3 |
| architecture-health-platform | not counted | 23 test files |

hebkit and schlauzug have no counted tests; see their sections.

### Rate-limit gaps
- `/suite/*` and `/platform/inbound` are merged after the limiter layers in eigenruhe and lesezeichen. kassensturz has no limiter on `/platform/inbound`.
- solo-leveling-system uses its own 300/min Postgres quota, not baukit-ratelimit. Its OAuth callback and OG image route have no limit at all.
- runtime-analyzer: anonymous `/auth/*` traffic shares one global key, so one client can lock everyone out of sign-in.
- schluesselbund: invalid `/platform/v1` tokens are unlimited.
- lesezeichen: the limiter is optional in production.
- LLM, export, upload and outbound routes get only the generic bucket almost everywhere.

### Sync isolation
- Holds (guarded upsert): eigenruhe, lesezeichen.
- Latent: kassensturz and hebkit have tables that rely on an unlocked pre-check. In tiefgang a foreign id makes the whole batch fail with a 5xx.
- Gap: redemut's device_id is not checked.
- Not used: solo-leveling-system, schluesselbund, schlauzug, architecture-health-platform.


---

# solo-leveling-system: tenancy and rate-limit audit

Backend: `/home/patrick/projects/solo-leveling-system/backend`. Paths below are relative to `backend/crates/` unless they start with `migrations/` or `~/.cargo`. Read-only audit, 2026-11-07.

## 1. Ownership model

There are two scopes:

- **User scope.** Most data is keyed by `user_id` (`user_quests`, `goals`, `decisions`, `experiments`, `lg_*`, `notifications`, `operations`, `suite_links` and others).
- **Organization scope.** Teams, cohorts, quest definitions and assignments, wellness metrics and org API keys are keyed by `organization_id`.

Guilds and parties are a third kind of group. Membership lives in `guild_members` and party members.

- **Principal type.** `AuthContext` (domain crate) is `User { id, role, org_context: Option<{org_id, role}>, suite_subject }`, `OrgApiKey` or `PublicApiKey`.
- **Extractor.** `Authenticated(pub AuthContext)` in `solo-leveling-system-api/src/auth/mod.rs:405`, with `FromRequestParts` at `:423`. It reads the `AuthContext` that the `auth::authenticate` middleware put into the request extensions. Each `v1/*.rs` router adds that middleware with `route_layer`, for example `v1/groups.rs:1297`.
- **User id helper.** `authenticated_user_id(&context)`, for example `v1/groups.rs:1303`.
- **Org gate.** `scoped_user()` in `v1/orgs.rs:1251`. First, `token_scoped_user()` at `:1227` requires the selected `X-Organization-Id` context to equal the path `orgId`. Then `organizations.active_role(path_org_id, user_id)` checks the role persisted in SQL. Every `/organizations/{orgId}/*` handler calls it.
- **Org API key gate.** `require_org_admin()` in `v1/api_keys.rs`.
- **Admin gate.** `admin_actor()` in `v1/admin.rs:1681` calls `auth::require_admin`, and all 36 admin handlers call it. The `admin_content`, `admin_ops` and `admin_economy` modules have their own checks. Admin routes are global, so they are left out of the tenancy table.

## 2. Route inventory

`openapi.json` has 204 operations with a path id. 29 are admin, which leaves 175. I mapped 150 of them to handlers with `#[utoipa::path]`. The other 25 are the 10 baukit-suite link routes, connection accept/decline variants, and handlers whose attribute blocks my script could not parse. I traced the main ones by hand.

Legend:
- **SQL**: the owner or org id is in the WHERE of the query that touches the id.
- **SVC**: the row is loaded by id alone, then compared to the caller in Rust (latent).
- **MISSING**: no owner, membership or visibility check anywhere on the path.
- **N/A**: global catalog id.

### 2a. `/me/*` (user scope)

| Routes | Handler | Repo / check | Class |
|---|---|---|---|
| GET/complete/reroll/tasks/progress `/me/quests/{id}[...]` | `v1/quests.rs:657,722,411,751,785` | `UserQuestService::get` calls `find_by_id_for_user(id,user)` (`services/src/quests.rs:845-853`). Updates use `WHERE id=$1 AND user_id=$2` (`postgres/src/user_quests.rs:317,571,679`) | SQL |
| GET `/me/categories/{id}` | `v1/categories.rs:240` | `get_for_user` calls `find_enrollment(user_id, id)` (`services/src/categories.rs:160-174`) | SQL |
| PATCH `/me/categories/{id}` | `v1/categories.rs:269` | `get_for_user` + `ensure_owned` (`services/src/categories.rs:234-235,438`), then `UPDATE categories ... WHERE id = $1` (`postgres/src/categories.rs:412-415`) | SVC |
| DELETE `/me/categories/{id}` | `v1/categories.rs:297` | `DELETE ... WHERE id=$1 AND owner_user_id=$2` (`postgres/src/categories.rs:465`) | SQL |
| PATCH active, POST priority | `v1/categories.rs:323,352` | `UPDATE user_categories ... WHERE id=$1 AND user_id=$2` (`postgres/src/categories.rs:536-539`) | SQL |
| `/me/connections/{id}` remove/accept/decline | `v1/social.rs:338` | `find_by_id` (`postgres/src/connections.rs:68`, `WHERE id = $1`), then `.filter(is_participant)` (`services/src/social/connections.rs:138-141`). The delete is `delete(connection_id, user_id)` | SVC |
| PUT equip, DELETE unequip `/me/cosmetics/...` | `v1/content.rs:817,838` | `find_ownership(user_id, item)` (`services/src/content.rs:669`) | SQL |
| `/me/goals/{id}` GET/PATCH/DELETE, `/me/decisions/{id}` GET/review | `v1/lifegraph/goals.rs:420,450,482,649,679` | `WHERE g.user_id=$1 AND ...g.id=$2`, `SELECT ... FOR UPDATE WHERE user_id=$1 AND id=$2`, upserts guarded by `WHERE goals.user_id=excluded.user_id` (`postgres/src/lifegraph/goals.rs:48,92,183-188,227,454,471,484-485`) | SQL |
| `/me/trials/{id}` GET, results | `v1/lifegraph/trials.rs:372,432` | `owned()` filters `list_trials(user)` (`WHERE user_id=$1`) (`services/src/lifegraph/trials.rs:60-71`; `postgres/src/lifegraph/trials.rs:325`). Follow-up queries use the id only | SQL (list-scoped) |
| `/me/lifegraph/events/{id}` GET/PATCH | `v1/lifegraph/insights.rs:486,508` | `WHERE id=$1 AND user_id=$2` (`postgres/src/lifegraph/events.rs:177,186`) | SQL |
| observations seen/dismiss, daily summary | `v1/lifegraph/observations.rs:159,181,203` | `postgres/src/lifegraph/observations.rs:282,299,408` | SQL |
| `/me/attributes/{attribute}`, metric series | `v1/lifegraph/attributes.rs:172`, `insights.rs:563` | keyed by user + key | SQL |
| `/me/arcs/{enrollmentId}` | `v1/liveops.rs:854` | `WHERE id = $1 AND user_id = $2` (`postgres/src/narratives.rs:640`) | SQL |
| `/me/boss-attempts/{id}` progress/complete | `v1/liveops.rs:1033,1050` | `postgres/src/narratives.rs:1102` | SQL |
| `/me/events/{eventId}` progress/chains/claim | `v1/liveops.rs:714,754,731` | `find_participation(user,event)` (`postgres/src/events.rs:470`), `list_chains(user_id, ...)` (`services/src/liveops.rs:453`) | SQL |
| `/me/story/steps/{stepId}/choice` | `v1/liveops.rs:944` | the step must be in the user's `current(user_id)` positions (`services/src/liveops.rs:1086-1091`) | SQL |
| `/me/rewards/{id}/use` | `v1/economy.rs:349` | `find_grant(grant_id, user_id)` (`services/src/economy.rs:328`) | SQL |
| `/me/life-domains/{id}` POST/PATCH/synergies | `v1/content.rs:901,924,946` | `find_user_domain(user_id, id)` (`services/src/content.rs:823,864`). POST adopts a global template | SQL |
| `/me/notifications/{id}/read`, `/channels/{configId}` | `v1/notifications.rs:240,328` | `mark_read(user_id, id)`, `delete_channel(user_id, id)` (`services/src/notifications.rs:177,301`) | SQL |
| `/me/operations/{id}` GET/DELETE | `v1/integrations.rs:439,474` | `find_for_user`, `cancel_pending(user, id)` (`services/src/operations.rs:84-110`) | SQL |
| `/me/public-api-keys/{keyId}` | `v1/api_keys.rs:240` | `list_public_keys(user_id)` filtered by id (`services/src/api_keys.rs:275-283`) | SQL (list-scoped) |
| `/me/guild-invites/{id}` accept/decline | `v1/groups.rs:854,871` | `find_invite(id)` (`postgres/src/guilds.rs:725`, `WHERE id = $1`), then `.filter(invite.user_id == user_id)` (`services/src/groups.rs:1066-1074`) | SVC |
| `/me/party-invites/{id}` accept/decline | `v1/groups.rs:1231,1248` | `find_invite` (`postgres/src/parties.rs:409`), then `.filter(invited_user_id == user_id)` (`services/src/groups.rs:1312-1316`, `:1356-1358`) | SVC |
| `/me/integrations/{provider}` connect/disconnect/sync/task-mappings | `v1/integrations.rs:341,395,414,491,516` | keyed by `(user_id, provider)` | SQL |
| GET `/me/integrations/{provider}/callback` (no bearer) | `v1/integrations.rs:362` | the user comes from the signed OAuth `state` (`services/src/integrations.rs:478-492`). Nothing binds it to the browser session | see Findings |
| `/me/suite/mappings/{sourceApp}` | `v1/suite.rs:219,264` | keyed by `user.id` | SQL |
| `/me/erasures/{operationId}` | `v1/erasure.rs:99` | `store().status(&subject, operation_id)` | SQL (subject-keyed) |
| `/public/me/quests/{id}` | `v1/api_keys.rs:404` | `UserQuestService::get(user_id, id)` | SQL |

### 2b. Guilds, parties, challenges, social

| Routes | Handler | Repo / check | Class |
|---|---|---|---|
| **GET `/guilds/{id}`** | `v1/groups.rs:625` (does not even take `Authenticated`) | `summary(id)` calls `active_guild` (`services/src/groups.rs:300,1015-1020`, `postgres/src/guilds.rs:265`). The only check is `disbanded_at`. Private guilds are hidden only from browse (`postgres/src/guilds.rs:292`) | **MISSING** (private guild read) |
| **GET `/guilds/{id}/members`** | `v1/groups.rs:704` | `members_page(id)` (`services/src/groups.rs:306-325`), no membership check | **MISSING** |
| **GET `/guilds/{id}/progression`, `/quests`, `/quest-templates`** | `v1/groups.rs:907,923,1053` | `progression`, `quests_page`, `quest_templates_page` (`services/src/groups.rs:370-410,685`) call only `active_guild` | **MISSING** |
| **GET `/guilds/{guildId}/competitions`, `/{competitionId}`** | `v1/contests.rs:568,623` | `list(guild_id)` and `get(competition_id)` (`services/src/contests.rs:556-591`). `get` checks only that the competition belongs to the path guild (`require_competition_guild`), not that the caller is a member | **MISSING** (private guild) |
| PATCH `/guilds/{id}`, invites POST/GET/DELETE, members PATCH/DELETE, quests POST, quest cancel/contribute, quest-templates POST | `v1/groups.rs:644,783,801,821,728,753,959,1038,997,1068` | `require_role(guild, actor, ...)` uses SQL `find_member(guild,user)` (`services/src/groups.rs:1023`). Child rows are then loaded by id (`find_invite`/`find_quest`, `postgres/src/guilds.rs:725,820`) and compared with `.filter(x.guild_id == guild_id)` (`services/src/groups.rs:606-609,933-936,1003-1006`). Writes are `WHERE id = $1` (`postgres/src/guilds.rs:335,694`) | SVC |
| GET `/guilds/{id}/feed`, `/leaderboard`, `/recaps` | `v1/guild_extensions.rs:277,218,240` | `require_member` / `require_role(Member)` (`services/src/groups.rs:847,880,884`) | SQL (membership) |
| POST `/guilds/{id}/join` | `v1/groups.rs:687` | private visibility returns 403 (`services/src/groups.rs:436`) | OK |
| DELETE `/guilds/{id}/members/me` | `v1/groups.rs:768` | keyed by `(guild,user)` | SQL |
| **GET `/parties/{id}`** | `v1/groups.rs:1115` (no `Authenticated`) | `find(id)` (`postgres/src/parties.rs:194-199`) + `list_members` (`services/src/groups.rs:1226-1237`), no membership check | **MISSING** (party and member list readable by any authenticated user) |
| POST `/parties/{id}/invites`, `/activate`, DELETE `/parties/{id}`, `/members/me` | `v1/groups.rs:1129,1147,1179,1164` | `get(id)` then `leader_user_id != actor` or member scan in Rust (`services/src/groups.rs:1256-1258` and the `activate`, `leave`, `disband` bodies) | SVC |
| `/challenges/{id}` GET/join/leave/progress/participants/results | `v1/contests.rs:441-549` | public by design. `find_challenge` is `WHERE id = $1` (`postgres/src/challenges_competitions.rs:181`), and progress/leave are keyed by `(challenge,user)`. The `guild_id` on create (`v1/contests.rs:61`, `services/src/contests.rs:144`) is not checked for membership | N/A (see Findings) |
| POST/DELETE `/feed/{entryId}/reactions` | `v1/social.rs:530,566` | `find_entry(id)`, then `ensure_reaction_access` covers guild membership plus connection visibility (`services/src/social/activity_feed.rs:186-191,232-246`) | SVC |
| `/arcs/{id}/enroll`, `/boss-quests/{id}/attempts`, `/events/{id}[...]`, `/story/...`, `/shop/{id}/purchase`, `/progression/season-pass/tiers/{id}/claim`, `/quests/{id}`, `/public/quests/{id}`, `/leaderboards/{board}`, `/users/{username}` | `v1/liveops.rs`, `content.rs`, `quests.rs`, `social.rs` | global catalog ids. The resulting writes are keyed by the caller's user id | N/A |

### 2c. Organizations (org scope; every handler calls `scoped_user`, `v1/orgs.rs:1251`)

| Routes | Handler | Repo / check | Class |
|---|---|---|---|
| GET org, members, report-snapshots, reports, wellness config GET/PUT/DELETE, list routes | `v1/orgs.rs:299,315,778,704,750,802-836` | `organization_id` is in the WHERE | SQL |
| teams `{teamId}` GET/PATCH/DELETE, members GET/POST/PATCH/DELETE | `v1/orgs.rs:366-481` | `TeamService::scoped`: `find_team(id)` (`postgres/src/teams.rs:114`, `WHERE id = $1`), then `.filter(team.organization_id == organization_id)` (`services/src/orgs.rs:366-372`). `save_team` and `delete_team` use `WHERE id = $1` only (`postgres/src/teams.rs:157,172`). The `team_members` FK `(organization_id,user_id) -> organization_members` blocks non-members (`migrations/0028_enterprise_organizations.sql:70-71`) | SVC |
| cohorts `{cohortId}` GET/PATCH/DELETE, members GET/POST/DELETE | `v1/orgs.rs:1049-1156` | `find_cohort` (`postgres/src/onboarding.rs:149`, `WHERE id=$1`), then `.filter` (`services/src/orgs.rs:1819-1829`). `save_cohort` and `save_member` use `WHERE id=$1` (`postgres/src/onboarding.rs:206,313`) | SVC (plus the side effect in Findings) |
| wellness metrics `{metricId}` PATCH/DELETE/records/aggregate | `v1/orgs.rs:910,945,969,986` | `find_metric` (`postgres/src/wellness.rs:239`, `WHERE id=$1`), then `.filter` (`services/src/orgs.rs:1536-1546`). `save_metric` uses `WHERE id=$1` (`postgres/src/wellness.rs:256`) | SVC |
| quest-definitions `{definitionId}` GET/PATCH/DELETE/self-enroll, quest-assignments `{assignmentId}` PATCH/DELETE | `v1/orgs.rs:535-688` | loaded from `list_quest_definitions(org)` / `assignments(org)` and filtered by id (`services/src/orgs.rs:737-747,883-888`). `save_quest_definition` and `save_quest_assignment` use `WHERE id=$1` (`postgres/src/organization_enterprise.rs:333,426`) | SQL read (list-scoped), id-only write |
| `/organizations/{organizationId}/api-keys/{keyId}` DELETE | `v1/api_keys.rs:188` | `list_org_keys(org)` filtered by id (`services/src/api_keys.rs:172-183`) | SQL (list-scoped) |

### 2d. Suite links (baukit-suite 0.18.2)

| Routes | Repo / check | Class |
|---|---|---|
| `/suite/links/{id}` GET/PATCH/DELETE, deliveries, reenable, replay, test | `SuiteLinkService::get` calls `find_link_for_owner`, which is `find_link(id)` + `.filter(link.user_id == owner)` (`~/.cargo/.../baukit-suite-0.18.2/src/adapters/postgres/mod.rs:539-548`). `find_link` first calls `refresh_failures(id)` and runs `SELECT ... WHERE id=$1` (`mod.rs:524-530`). `disconnect` also compares `link.user_id != owner` (`src/services/link.rs:569`) | SVC (in the baukit crate) |
| `/suite/inbound/{linkId}`, `/links/{remoteLinkId}/revoke`, `/links/requests/{id}/complete` | HMAC or link code, no bearer token | protocol auth |

## 3. Tests (behavior search: a second principal hits an id route)

These tests make a second user or org call an id route and expect 403/404 or empty data:

- `/me/operations/{id}`: `solo-leveling-system-api/tests/integrations_endpoints.rs:221,256`
- `/me/quests/{id}/reroll`: `tests/lifegraph_adaptive_quests.rs:700-707`
- `/me/trials/{id}`: `tests/lifegraph_trials.rs:308,366-379`
- observations seen/dismiss: `tests/lifegraph_observations.rs:497`
- goals: `tests/lifegraph_goals.rs:332-339`. Decisions: `tests/lifegraph_goals.rs:1223`
- attributes: `tests/attributes.rs:224-262`
- suite mappings: `tests/suite/lifegraph.rs:245,272`
- suite links: `tests/suite/regressions.rs:100`
- season tier claims: `tests/content_persistence.rs:726`
- feed reactions with a second user: `tests/social_endpoints.rs:81,140-150`
- org path mismatch (`/organizations/{other}/teams` returns 403): `tests/hub_organizations.rs:602-621`
- competitions, non-leader: `tests/contests_flows.rs:281-300`
- private guild join returns 403: `tests/groups_endpoints.rs:287,312`

**Numbers.** Of the 175 non-admin id routes, about 20 have a test that drives them with a second principal. That counts the goal and decision routes, the trial, observation and attribute routes, and the org team list path.

**Gaps:**
- No test reads a private guild or party by id as an outsider. `groups_endpoints.rs` creates a private guild at `:294` but only checks join.
- No test sends org A's path with org B's team, cohort, metric or definition id. The tests in `orgs_enterprise.rs:1131,1140,1178` use random UUIDs.
- No cross-user test covers guild/party invites, connections, or categories PATCH.

## 4. Rate limiting

- **Global per-user quota.** All bearer routes go through it. `auth::authenticate` calls `user_quota.consume_user_quota(user_id)` (`solo-leveling-system-api/src/auth/mod.rs:277-291`), with a Postgres fixed window in `postgres/src/api_keys.rs:114-142` and a default of 300/min (`solo-leveling-system-bin/src/settings.rs:240`). It is attached via `.with_user_quota(...)` on the v1 router, the suite protocol router and the plan router (`solo-leveling-system-api/src/lib.rs:84-105`). It is not baukit-ratelimit.
- **API key quota.** `/public/*` and org keys use `api_key_quota_exceeded` (`auth/api_key.rs:111-118`).
- **baukit-ratelimit.** MCP uses Redis when configured, in memory otherwise (`solo-leveling-system-bin/src/compose/mcp.rs:124-127`). Redis is required outside local (`settings.rs:867-870`). Suite protocol uses `InMemoryRateLimitStore` (`compose/suite.rs:49`). That limiter is per process, so it is not shared across replicas.
- **Route-specific limits.**
  - AI: a daily per-user call cap (`services/src/ai.rs:791-794`, returns 429 via `v1/ai.rs:402`) plus suite credits.
  - Competition score refresh returns 429 (`v1/contests.rs:817-818`).
  - Integration sync goes through idempotency and the operation queue.

**Abuse-prone routes with no limit beyond the generic 300/min per user:**
- invite routes `POST /guilds/{id}/invites`, `POST /parties/{id}/invites`, `POST /me/connections` (connection requests)
- `POST /me/referral/redeem`
- `POST /me/categories/import`
- `POST /me/health/sync`
- `POST /me/integrations/{provider}/connect`
- `GET /guilds?search=`
- `POST /organizations/{orgId}/reports/{reportType}/snapshots` and `GET .../reports/{reportType}` (report generation)
- `POST /me/hunter-reports/generate` (covered by the AI cap only)

**Unauthenticated routes with no limiter at all:**
- `GET /api/v1/me/integrations/{provider}/callback`, which makes an outbound token exchange per call
- `GET /api/og/hunter/{username}` (`open_graph.rs:15`, SVG render)
- `GET /api/v1/system/status`

Sign-in and sign-up are handled by the external hub/OIDC, so the product has no local auth routes.

## 5. Sync

The product does not use baukit-sync. There is no `baukit_sync` dependency in `Cargo.toml` or any crate. Suite delivery is the nearest equivalent. Inbound events are applied for the link's owner (`postgres/src/suite/outbox.rs:440`, `WHERE user_id=$1`). Lifegraph event upserts are keyed by `(user_id, source_app, source_event_id)` (`postgres/src/lifegraph/events.rs:97`), so one user's pushed event cannot overwrite another user's row.

## Findings

### Confirmed missing scoping

1. **Private guild data is readable by any authenticated user who has the id.** Affects `GET /guilds/{id}`, `/members`, `/progression`, `/quests`, `/quest-templates`, and `/guilds/{guildId}/competitions[/{competitionId}]`. Handlers: `v1/groups.rs:625,704,907,923,1053`, `v1/contests.rs:568,623`. The services check only `disbanded_at` (`services/src/groups.rs:1015-1020`). Visibility is enforced only in browse SQL (`postgres/src/guilds.rs:292`) and on join (`services/src/groups.rs:436`).
2. **`GET /parties/{id}` returns the party and its full member list to any authenticated user** (`v1/groups.rs:1115`, `services/src/groups.rs:1226-1237`).
3. **`POST /organizations/{orgId}/onboarding/cohorts/{cohortId}/members` has a cross-tenant side effect.** An org Admin can pass any `userId`. `add_member` calls `arcs.enroll(user_id, cohort.quest_arc_id)` (`services/src/orgs.rs:1934-1956`) before the cohort-member insert. The FK to `organization_members` (`migrations/0030_enterprise_wellness_platform.sql:90-91`) rejects that insert for non-members, but the enrollment was already committed in a separate call. A non-member user ends up enrolled in the org's quest arc. I found this by reading the code. No test exercises it.
4. **The OAuth integration callback binds the provider account to the user in `state`, not to the browser session** (`v1/integrations.rs:355-388`, `services/src/integrations.rs:478-492`). A victim who completes an authorize URL that an attacker started links the victim's provider data to the attacker's account (OAuth CSRF). Whether this is exploitable depends on whether `state` is single-use or tied to the browser, which I did not verify.
5. Minor: `POST /challenges` accepts any `guildId` without a membership check (`v1/contests.rs:61`, `services/src/contests.rs:144`), so anyone can label a challenge with someone else's guild.

### Service-check only (latent)

- **Org:** teams (`postgres/src/teams.rs:114,157,172`, filter at `services/src/orgs.rs:366-372`), onboarding cohorts (`postgres/src/onboarding.rs:149,206,313`, filter at `services/src/orgs.rs:1819-1829`), wellness metrics (`postgres/src/wellness.rs:239,256`, filter at `services/src/orgs.rs:1536-1546`). Quest definitions and assignments are read through an org-scoped list but written with id-only `UPDATE` (`postgres/src/organization_enterprise.rs:333,426`).
- **Guilds:** invites, quests and guild update (`postgres/src/guilds.rs:265,335,694,725,820`). Parties and party invites (`postgres/src/parties.rs:194,409`). Guild and party invite accept/decline.
- **Social:** connections remove/accept/decline (`postgres/src/connections.rs:68`). Feed reactions (`find_entry`).
- **Categories:** PATCH `/me/categories/{id}` (`postgres/src/categories.rs:412-415`, `UPDATE ... WHERE id = $1`).
- **Suite links in baukit-suite:** `find_link_for_owner` filters in Rust (`adapters/postgres/mod.rs:539-548`). Before that filter runs, it calls `refresh_failures(id)` on any link id.

### SQL-scoped (good)

- All user quest, lifegraph, goal, decision, trial, observation, event, attribute, arc, boss, event-participation, reward-grant, cosmetic, life-domain, notification, operation, public-key, integration and suite-mapping routes. They put `user_id` in the WHERE, use upserts guarded by an owner `WHERE` clause, or filter an owner-scoped list.
- Org-level reads and config, which bind `organization_id` and are gated by `scoped_user` with a persisted role check.
- Guild member-gated routes (feed, leaderboard, recaps) and every officer/leader action, which check membership in SQL through `find_member`.

---

# kassensturz: tenancy and rate-limit audit

Paths are relative to `/home/patrick/projects/kassensturz/backend/` unless they start with `baukit/`, which means `/home/patrick/projects/baukit/rust/crates/`. Read-only audit. Nothing in either repo changed.

## 1. Ownership model

- Tenant key: `user_identities.user_id UUID` (`migrations/0002_create_user_identities.sql:1-5`). The OIDC `subject` is unique there. Every product table carries `user_id` (or `owner_id` for `api_tokens`, `push_devices`, `push_installations`). Households add a sharing layer (`households`, `household_members`, `household_shares`, `household_grants`, `household_deliveries`) on top of per-user rows. They are not a second tenant key.
- Principal extractor: `baukit_auth::Principal` (`baukit/baukit-auth/src/axum_integration.rs:147`, `impl FromRequestParts for Principal`). `compose_api` installs `baukit_auth::establish_principal` as the outermost layer (`crates/kassensturz-bin/src/token_routes.rs:34-38`).
- Subject to user id: `UserService::resolve_subject` (`crates/kassensturz-services/src/lib.rs:48`). Handlers call it through per-module `owner()` helpers:
  - sync: `crates/kassensturz-api/src/sync.rs:154-174`. `X-Partition-Id` is only checked when the header is present (`:164`).
  - aggregator: `crates/kassensturz-api/src/aggregator.rs:74-89`. `x-kassensturz-partition` is mandatory and must equal `user.id`.
- Session gates: each feature router has a `session_only`/`session`/`authorize` route_layer that rejects PATs and fenced (erased) subjects, for example `crates/kassensturz-api/src/contract.rs:46-69`. The `erasure_fence` route_layer sits on the core router (`crates/kassensturz-api/src/lib.rs:95`, `:237`).
- DB-level backstop: child tables use composite `(user_id, parent_id)` foreign keys, so a row cannot reference another user's parent:
  - `migrations/0013_transactions.sql:45,65,79`
  - `migrations/0008_accounts.sql:40`
  - `migrations/0017_savings_goals.sql:40`
  - `migrations/0025_holdings.sql:21`
  - `migrations/0021_contracts.sql:43`
  - `migrations/0034_aggregator.sql:54`

  Exception: `savings_goals.linked_account_id` has no FK (`migrations/0017_savings_goals.sql:9`). The sync handler checks it against the owner instead (`crates/kassensturz-postgres/src/sync/goal.rs:149`).

## 2. Route inventory (authenticated routes that take a resource id)

The source is `openapi.json` (66 operations, 30 with path ids) plus the routers in `crates/kassensturz-api/src/*.rs` and `crates/kassensturz-bin/src/bin/api.rs:288-430`.

| Route | Handler | Repository / SQL | Class |
|---|---|---|---|
| `POST /api/v1/banking/aggregator/{id}/start` | `aggregator.rs:231` | `kassensturz-postgres/src/aggregator.rs:350` (`WHERE user_id=$1 AND id=$2 FOR UPDATE`), insert `:379` `ON CONFLICT(id) DO NOTHING`, pending-consent insert `:386` | SQL-scoped, with a cross-tenant DoS edge (see Findings) |
| `POST .../aggregator/{id}/callback` | `aggregator.rs:274` | consume `:399` (`connection_id=$1 AND user_id=$2`), promote `:583,:593` | SQL-scoped |
| `GET .../aggregator/{id}/refresh` | `aggregator.rs:343` | `kassensturz-postgres/src/imports/banking.rs:77` (`WHERE user_id=$1 AND id=$2`) | SQL-scoped |
| `POST .../aggregator/{id}/refresh` | `aggregator.rs:298` | `reserve_foreground_in` `aggregator.rs:681,690,699` and `load` `:468`, all `user_id=$1` | SQL-scoped |
| `DELETE /api/v1/banking/aggregator/{id}` | `aggregator.rs:376` | `disconnect_in` `aggregator.rs:504,520,521,527,534,545` | SQL-scoped |
| `POST /api/v1/contracts/proposals/{id}/confirm`, `/ignore` | `contract.rs:178`, `:188` to `decide` `:137` | `ContractService::decide` `kassensturz-services/src/contract.rs:193-213` opens `repository.begin(owner)`. The proposal is resolved from the owner's snapshot (`kassensturz-postgres/src/contract.rs:454`). `save` re-reads with `get_contract(tx, owner, id)` (`:533-541`), so a foreign `contractId` returns Conflict | SQL-scoped |
| `GET /api/v1/households/{id}` | `household.rs:150` | service `authorize` `kassensturz-services/src/household.rs:181-195` plus `reads.rs:38` (`id=$1 AND EXISTS(member user_id=$2)`) | SQL-scoped (double-checked) |
| `POST /api/v1/households/{id}/invitations` | `household.rs:166` | `mutations.rs:53-54`: `lock_household` (`household.rs:104`, membership in WHERE) plus `require_owner` | SQL-scoped |
| `DELETE /api/v1/households/{id}/invitations/{invitation}` | `household.rs:247` | `mutations.rs:84-86` (`household_id=$1 AND id=$2` after lock and owner check) | SQL-scoped |
| `POST .../households/{id}/leave`, `DELETE .../members/{member}`, `PATCH .../members/{member}/role`, `DELETE /api/v1/households/{id}` | `household.rs:260,286,299,273` | `change` `mutations.rs:106` takes `lock_household`, then `apply_change` `:245-251` / `change_member` `:276,:288,:296`, all keyed by `household_id` of the locked, membership-checked household | SQL-scoped |
| `POST /api/v1/imports/{id}/commit` | `imports.rs:232` | `kassensturz-postgres/src/imports.rs:483` (`id=$1 AND user_id=$2`), batch `:508` | SQL-scoped. The preview idempotency probe at `:342-363` loads by id and then compares |
| `DELETE /api/v1/me/api-tokens/{id}` | `api_tokens.rs:336` | `TokenService::revoke` `kassensturz-services/src/api_tokens.rs:99-107` to `baukit/baukit-auth/src/postgres.rs:137-139` (`owner_id=$1 AND id=$2`) | SQL-scoped |
| `DELETE /api/v1/push/devices/{id}` | `push.rs:206` | `kassensturz-postgres/src/notifications.rs:349-351` (`owner_id`) | SQL-scoped |
| `POST /api/v1/push/devices` (body id) | `push.rs:141` | upsert `notifications.rs:322` `ON CONFLICT(id) ... WHERE push_installations.owner_id=EXCLUDED.owner_id` | SQL-scoped. Token reassignment `:314` is not owner-scoped, which looks intentional because a device token has one owner |
| `GET /api/v1/suite/links/{id}` | baukit `services/link.rs:535` | `find_link_for_owner` `baukit/baukit-suite/src/adapters/postgres/mod.rs:539-548` loads `WHERE id=$1` and then filters `user_id` in Rust | Service-check only |
| `GET .../links/{id}/deliveries` | baukit `adapters/postgres/mod.rs:788` | same `find_link_for_owner` (`:794`), then job_outbox by `link_id` | Service-check only |
| `POST .../links/{id}/replay`, `/test` | baukit `services/delivery.rs:110`, `:150` | `find_link_for_owner` (`delivery.rs:126,159`) | Service-check only |
| `DELETE /api/v1/suite/links/{id}` | baukit `services/link.rs:560-580` | `get` (Rust filter), then `link_for_update` with Rust compare `:569`, then `revoke_in_transaction` `mod.rs:516` `WHERE id=$1` only | Service-check only |
| `PATCH /api/v1/suite/links/{id}`, `POST .../reenable` | baukit `mod.rs:571`, `:611` | UPDATE `WHERE id=$1 AND user_id=$2` | SQL-scoped |
| `POST /api/v1/suite/links/requests/{id}/complete` | baukit `services/link.rs:118` | `mod.rs:212-221` (`id=$1 AND user_id=$2`) | SQL-scoped |
| `POST /api/v1/suite/inbound/{linkId}`, `POST .../links/{remoteLinkId}/revoke` | baukit peer endpoints | HMAC-signed peer calls, not user-tenant | n/a |
| `GET /me/erasures/{operationId}` | `lib.rs:327` | `baukit/baukit-erasure/src/store.rs:217` (`id=$1 AND subject_hash=$2`) | SQL-scoped |
| `GET/PUT/DELETE /items/{id}`, `GET/POST /items` | `lib.rs:400-530` (`_principal` unused) | `kassensturz-postgres/src/lib.rs:66,81,98,138,151`. The table has no owner column (`migrations/0001_create_items.sql:1-5`) | **MISSING.** This is a global table that any authenticated user can read, rename and delete (template leftover, still called by `mobile/src/api.ts:41`) |
| `POST /api/v1/sync/push` (body entity ids) | `sync.rs:176` | see section 5 | Mostly service-check only (pre-check, then unguarded upsert) |
| `GET /api/v1/sync/pull` | `sync.rs` | `kassensturz-postgres/src/sync.rs:569-594`, owner-scoped `pull_tables` | SQL-scoped |
| `POST /api/v1/budgets/restore-results` (body ids) | `budget_restore.rs` | owner-scoped reads `kassensturz-postgres/src/budget_restore.rs:131,159,197`. The global PK probe `:100-101` is a deliberate cross-tenant existence check | SQL-scoped (existence oracle only) |
| `POST /api/v1/backup/restore-sessions[/complete]` | `budget_restore.rs:39-43` | `restore_session.rs:10,36,62,76` (`user_id`) | SQL-scoped |
| `POST /api/v1/push/budget-crossings` (installation header) | `push.rs:229` | `notifications.rs:437` (`id=$1 AND owner_id=$2`) | SQL-scoped |
| MCP tools (`/mcp`) | `kassensturz-bin/src/agent.rs` | `kassensturz-postgres/src/agent.rs:104-225` and `agent/reads.rs`, all `user_id`/`owner`. Writes go through the same sync repository | SQL-scoped (writes as sync) |

Other unscoped by-id reads on the path, each reached only after an owner check:
- `sync/goal.rs:125` (`stored_goal` `WHERE id=$1`)
- `sync/sharing.rs:50-58` (`target_owner` `SELECT user_id FROM <table> WHERE id=$1`)
- `sync/sharing.rs:114` (`goal_contributions WHERE id=$1`)

Within `push_foreign` the write is authorized by the `household_shares` + `household_members` EXISTS at `sync/sharing.rs:135`, and deletes are refused (`:110`).

## 3. Tests (cross-tenant behaviour)

I counted 29 user-authenticated path-id operations (30 in OpenAPI, minus the 2 peer-signed suite routes, plus the 3 `/items/{id}` methods, which OpenAPI lists under one path). Six have a genuine second-user test:

| Route | Test |
|---|---|
| aggregator callback | `tests/aggregator.rs:143-146` (`factory.for_owner(other).continue_step`, service level) |
| households GET | `tests/households.rs:150-153` (outsider gets NotFound, service level) |
| api-token revoke | `tests/api_tokens.rs:205` (bob revokes alice's token, NotFound) |
| push device unregister and register | `tests/review_m63.rs:326-345` (store level) |
| suite link GET and `/deliveries` | `tests/agent_tools/sharing_access.rs:445-458` (foreign link returns 404 over HTTP) |

Partial coverage:
- Household invite, role and remove are tested with a non-owner member, not an outsider (`tests/households.rs:403-430`).
- The HTTP GET sweep `tests/agent_tools/sharing_access.rs:363-460` hits every GET in `openapi.json` with a foreign transaction id. For `aggregator/{id}/refresh` and `me/erasures` that id is not a real foreign resource, so the 404 proves little.

No cross-tenant test exists for:
- aggregator start, POST refresh and disconnect
- contracts confirm and ignore
- imports commit
- household invitation revoke and delete by an outsider
- suite PATCH, DELETE, reenable, replay and test
- `/items/{id}`, which would fail one anyway

Sync push cross-owner tests (`entity_id_unavailable` and similar):
- `tests/accounts_integration.rs:213`
- `tests/goals_integration.rs:224`
- `tests/holdings_integration.rs:51`
- `tests/transactions_integration.rs:43`
- `tests/rules.rs:221`
- `tests/contracts_integration.rs:911`
- `tests/imports.rs:1037`
- `tests/budgets.rs:268` (concurrent UUID collision)
- `tests/budgets/restore.rs:149,566`

I found none for `balance_snapshot`, `goal_contribution`, `transaction_split` or `contract_occurrence`.

## 4. Rate limiting

- **Global baukit-ratelimit.** `baukit_ratelimit::layers` wraps everything merged up to `crates/kassensturz-bin/src/bin/api.rs:376`: core, suite, push, restore, dedupe, rules, imports, contracts, forecast, aggregator, token management and households. The identity scope defaults to 60/min plus 10 burst, the IP scope to 6000/min (`baukit/baukit-config/src/lib.rs:206-222`). It is attached only when `RedisRateLimitStore::connect_if_enabled` returns a store (`api.rs:103`, `:362`). The identity scope works because `establish_principal` runs outside it (`token_routes.rs:34-38`).
- **`item_writes` group.** 30/min per subject on POST, PUT, PATCH and DELETE (`api.rs:34-35`, `:363-375`, `:493-502`).
- **PAT limits.** Failed PATs are limited to 30/min per IP. Authenticated PATs get 600/min plus 120 burst (`token_routes.rs:8-27`, `crates/kassensturz-api/src/api_tokens.rs:30-35,66-90`). Without Redis this falls back to an in-memory store.
- **Imports.** Per-owner quota on preview and commit (`crates/kassensturz-api/src/imports.rs:262-284`).
- **Households.** Per-subject quota on invitation create, preview and accept (`crates/kassensturz-api/src/household.rs:87-110`).
- **Suite.** Per-link inbound limit and exchange-failure limit (`baukit/baukit-suite/src/services/ingest.rs:143-151`, `services/link.rs:377`).
- **MCP.** Per-token `TokenQuota` (`crates/kassensturz-mcp/src/quota.rs:14-22`, `api.rs:390-396`).
- **Aggregator.** Domain caps, not a limiter: a connection count (`aggregator.rs:261-267`), a foreground refresh window (`kassensturz-postgres/src/aggregator.rs:674-708`) and a daily allowance (`:86-120`).

Abuse-prone routes with only the global identity/IP bucket:
- `POST .../aggregator/{id}/start`, an outbound call to the aggregator provider (60/min per user, no per-route quota)
- `GET /api/v1/banks` (search)
- `POST /api/v1/rules/preview`
- `GET /api/v1/forecast`
- `POST /api/v1/sync/push`
- `DELETE /me` (erasure)
- `POST /api/v1/backup/restore-sessions`
- `POST /api/v1/suite/links/{id}/test` (outbound peer call)
- `POST /api/v1/push/budget-crossings`, which triggers push sends

`/platform/inbound` is merged after the limiter (`api.rs:430`). It has no rate limit, only HMAC verification (`crates/kassensturz-api/src/platform.rs:155-165`).

## 5. Sync (push/pull)

Push enters `push_changes` (`kassensturz-postgres/src/sync.rs:500-530`). With households disabled it rejects a foreign `owner_id` (`:510`). With households enabled, `push_foreign` (`sync/sharing.rs:98`) resolves the stored owner and requires an editor share (`:135`).

Same-owner handlers run a pre-check `EXISTS(... WHERE id=$1 AND user_id<>$2)` and reject with `entity_id_unavailable`:

| Entity | Pre-check |
|---|---|
| account | `account.rs:43` |
| balance_snapshot | `balance_snapshot.rs:31` |
| holding | `holding.rs:36` |
| goal_contribution | `goal_contribution.rs:32` |
| goal | `goal.rs:93` |
| contract | `contract.rs:54` (via `:469-476`) |
| contract_occurrence | `contract.rs:174` |
| transaction | `transaction.rs:244` |
| split | `transaction.rs:335` |
| category_rule | `category_rule.rs:33` |
| budget | `budget.rs:126` |
| share | `share.rs:33` |

Settings require `entity_id == owner` (`settings.rs:30`). Bank connections are server-owned and always rejected (`bank_connection.rs:28`).

The upserts that follow a pre-check fall into two groups.
- **Owner guard in SQL.** These add `ON CONFLICT ... WHERE <t>.user_id=excluded.user_id`:
  - `transaction.rs:509,935`
  - `category_rule.rs:74`
  - `budget.rs:54,69`
  - `share.rs:92`
  - `notifications.rs:322`
- **No owner guard.** These run `ON CONFLICT(id) DO UPDATE` with no guard:
  - `account.rs:117,171`
  - `balance_snapshot.rs:114,133`
  - `goal.rs:68,78`
  - `goal_contribution.rs:65,71`
  - `holding.rs:66,73`
  - `settings.rs:69`
  - `transaction.rs:560` (splits), `:572` (transaction tombstone)
  - `contract.rs:341,359,373,581`

  For these, isolation depends on the pre-check alone. The pre-check is not a lock against a concurrent insert of the same id by another user. Exploiting that needs the victim's UUID at creation time, so it is latent. `user_id` is never overwritten, but the victim's row data would be. The composite FKs stop cross-owner references.

Pull is SQL-scoped by owner (`sync.rs:569-594`, `:1146-1160`).

## Findings

### Confirmed missing scoping
1. `/items` and `/items/{id}` (GET, PUT, DELETE, plus list and create) run on a global `items` table with no owner column. Any authenticated user can read, rename and delete every row (`crates/kassensturz-api/src/lib.rs:400-530`, `crates/kassensturz-postgres/src/lib.rs:66-151`, `migrations/0001_create_items.sql`). This is a template leftover and the mobile client still calls it (`mobile/src/api.ts:41`). Remove it or scope it.

### Service-check only (latent)
2. Sync push upserts without an owner guard in `ON CONFLICT` (list in section 5). Isolation relies on a separate `EXISTS user_id<>owner` pre-check, which leaves a TOCTOU window. Adding `WHERE <t>.user_id=excluded.user_id` to each upsert would make the SQL carry the scoping, as `transactions`, `budgets` and `category_rules` already do.
3. Suite links GET, `/deliveries`, `/replay`, `/test` and DELETE (baukit-suite) load by id and then filter `user_id` in Rust:
   - `baukit/baukit-suite/src/adapters/postgres/mod.rs:539-548`
   - revoke `:516` `WHERE id=$1`
   - compare at `services/link.rs:569`

   `find_link` also runs `refresh_failures(id)` (`mod.rs:523-524`, `:1021-1029`) before the owner filter. That is a benign accounting write on a foreign id.
4. Aggregator start on another user's connection id: the `bank_connections` insert is `ON CONFLICT(id) DO NOTHING` (`kassensturz-postgres/src/aggregator.rs:379`). `aggregator_pending_consents` is keyed by `connection_id` alone (`migrations/0046_consent_expiry.sql:30`). An attacker who knows a victim's connection UUID can therefore park a pending consent that blocks the victim's re-consent (`:386` insert conflicts, `clear_pending` `:559` only deletes the caller's rows) until it expires. That is a low-severity cross-tenant DoS with no data exposure.
5. Existence oracles, each load-by-id then compare: the import idempotency probe (`kassensturz-postgres/src/imports.rs:342-363`) and the budget-restore global PK probe (`budget_restore.rs:100-101`). The sync `target_owner` lookup (`sync/sharing.rs:50`) is the same pattern but is followed by the share check.
6. Sync `X-Partition-Id` is optional (`crates/kassensturz-api/src/sync.rs:164`). The aggregator requires its partition header.

### SQL-scoped (good)
- Aggregator (all 5 id routes), contracts confirm and ignore, all household routes (membership in the `lock_household` WHERE plus an owner check in the transaction), imports commit, api-token revoke, push device register and unregister, budget crossings, restore sessions, erasure status, suite PATCH, reenable and complete, sync pull, MCP reads.
- Composite `(user_id, id)` FKs back every child table except `savings_goals.linked_account_id`, which the sync handler checks.

### Test gaps
- 6 of 29 path-id routes have a real cross-tenant test.
- Untested: aggregator start, POST refresh and disconnect; contracts confirm and ignore; imports commit; household revoke and delete by an outsider; suite PATCH, DELETE, reenable, replay and test; sync for balance_snapshot, goal_contribution, split and contract_occurrence.

### Rate-limit gaps
- `/platform/inbound` has no limiter.
- Aggregator `start` (outbound), `/api/v1/banks` search, rules preview, forecast, suite link `test` (outbound), push budget-crossings, `DELETE /me` and restore sessions rely only on the global per-identity bucket (60/min).

---

## lesezeichen

Root: `/home/patrick/projects/lesezeichen/backend`. All paths below are relative to it unless they start with `baukit/` (= `/home/patrick/projects/baukit/rust/crates/`).

### 1. Ownership model

- Data is scoped per **profile** (`profiles.id`, `Uuid`). Every tenant table carries `owner_id uuid` (e.g. `migrations/0010_library.sql`, `collection_items(owner_id,...)` at :73). No org/workspace layer.
- Principal: `baukit_auth::Principal`, established by `baukit_auth::establish_principal` as a route layer over the whole product API (`crates/lesezeichen-bin/src/bin/api.rs:440-443`). API tokens resolve to the owner's OIDC subject in `crates/lesezeichen-api/src/token_auth.rs:26-61`.
- Owner resolution: `identity::scoped_profile` (`crates/lesezeichen-api/src/identity.rs:8-31`) checks the route scope, resolves `Profile` from the subject and returns `profile.id`. Handlers pass that `Uuid` as `owner` into services and repositories. Route-level scope gate: `scopes::enforce` (`crates/lesezeichen-api/src/scopes.rs:182-195`, attached at `api.rs:420`). `TemplateOnly` routes (`/items*`) are only denied for API tokens. Any interactive or OAuth session passes (`scopes.rs:191-192`).
- MCP: the principal subject is parsed as the owner UUID (`crates/lesezeichen-mcp/src/lib.rs:218-229`) and tools call the same owner-taking services (`lib.rs:66-162`).

### 2. Route inventory (authenticated routes that take a resource id)

Repository convention: almost every read goes through `read_row` (`crates/lesezeichen-postgres/src/library.rs:384-398`, `WHERE owner_id=$1 AND id=$2`) and every library write goes through `write_row` (`library.rs:479-530`, `ON CONFLICT (id) DO UPDATE ... WHERE t.owner_id=$2`).

| Route | Handler | Repo SQL | Class |
|---|---|---|---|
| GET/PUT/DELETE `/items/{id}` | `lesezeichen-api/src/lib.rs:302,356,383` (`_principal` unused) | `lesezeichen-postgres/src/lib.rs:119` `SELECT ... FROM items WHERE id=$1`, `:176` `UPDATE items ... WHERE id=$1`, `:189` `DELETE FROM items WHERE id=$1`. Table has no owner column (`migrations/0001_create_items.sql`) | **MISSING** |
| GET `/items` | `lib.rs:275` | `postgres/src/lib.rs:104` lists all rows | **MISSING** (no id, but global list) |
| GET `/api/v1/documents/{id}` | `api/src/library.rs:481` | `LibraryService::get` -> `read_row` `postgres/src/library.rs:391` | SQL-scoped |
| DELETE `/api/v1/documents/{id}` | `api/src/library.rs:539` | `postgres/src/library.rs:1095-1127` (`read_row` + `write_row` + `delete_dependents` :831 all `owner_id=$1`) | SQL-scoped |
| GET `/api/v1/documents/{id}/content` | `api/src/library.rs:505` | `postgres/src/library.rs:1177` (`c.owner_id=$1 AND c.document_id=$2`) | SQL-scoped |
| GET `/api/v1/documents/{id}/bundle` | `api/src/agent.rs:199` | agent service -> owner-taking library port (`list_annotations` `library.rs:1145`) | SQL-scoped |
| GET `/api/v1/documents/{id}/file` | `api/src/uploads.rs:208` | `services/src/uploads.rs:139` -> `postgres/src/uploads.rs:278` (`u.owner_id=$1 AND u.id=$2`) | SQL-scoped |
| POST `/api/v1/documents/uploads/{id}/complete` | `api/src/uploads.rs:186` | `services/src/uploads.rs:94` -> `postgres/src/uploads.rs:135`, `:157` (`FOR UPDATE`), `:262` | SQL-scoped |
| POST `/api/v1/documents/{id}/finish`, `/cards/generate` | `api/src/generate.rs:127,160` | `postgres/src/generate.rs:162-240` (`live(&mut tx, owner, document)`, replay table `owner_id=$1`, `write_row`) | SQL-scoped |
| PATCH `/api/v1/cards/{id}` | `api/src/cards.rs:241` | `services/src/cards.rs:195` `library.get(owner, Cards, id)` -> `read_row`; write via `push` | SQL-scoped |
| DELETE `/api/v1/words/{id}`, POST `/words/{id}/retry` | `api/src/words.rs:164,181` | `services/src/words.rs:98,123` `library.get(owner, Words, id)`; write via `push` | SQL-scoped |
| DELETE `/api/v1/feeds/{id}` | `api/src/feeds.rs:193` | `postgres/src/feeds.rs:198` (`WHERE owner_id=$1 AND id=$2`) | SQL-scoped |
| DELETE `/api/v1/tokens/{id}` | `api/src/tokens.rs:293` | `baukit/baukit-auth/src/postgres.rs:137-139` (`WHERE owner_id=$1 AND id=$2`) | SQL-scoped |
| GET `/api/v1/imports/{id}` | `api/src/imports.rs:189` | `postgres/src/imports.rs:153` | SQL-scoped |
| GET `/api/v1/exports/{id}`, `/report`, `/file` | `api/src/export_schlauzug.rs:240,259,279` | `postgres/src/export_schlauzug.rs:260` (`owner_id=$1 AND id=$2 FOR UPDATE`) | SQL-scoped |
| GET `/api/v1/exports/redemut/{id}` | `api/src/push_redemut.rs:141` | `postgres/src/redemut.rs:232` -> `progress` `:108` | SQL-scoped |
| GET `/api/v1/briefings/{id}/audio` | `api/src/briefing.rs:147` | `services/src/briefing.rs:133` -> `postgres/src/briefing.rs:321`; blob grant keyed by owner+attempt | SQL-scoped |
| GET `/api/v1/me/erasures/{operationId}` | `api/src/lib.rs:243` | `baukit/baukit-erasure/src/store.rs:217` (`id=$1 AND subject_hash=$2`) | SQL-scoped |
| POST `/api/v1/sync/push` (row ids, parent ids in body) | `api/src/library.rs:611` | `apply_change` `postgres/src/library.rs:621-800`; parents `live_parent` `:539`; write `:498` | SQL-scoped (see 5) |
| POST `/api/v1/notes` (documentId, highlightId) | `api/src/agent.rs:236` | push path, `parents_exist` `postgres/src/library.rs:550-598` | SQL-scoped |
| POST `/api/v1/cards` (documentId / highlightId) | `api/src/cards.rs:190` | `services/src/cards.rs:110,140` `library.get(owner, ...)` | SQL-scoped |
| POST `/api/v1/words` (documentId) | `api/src/words.rs:130` | push path, `parents_exist` (Word -> Documents) `library.rs:565` | SQL-scoped |
| POST `/api/v1/imports` (uploadId) | `api/src/imports.rs:168` | `postgres/src/imports.rs:121` (`UPDATE uploads ... WHERE owner_id=$1 AND id=$2`) | SQL-scoped |
| POST `/api/v1/exports/schlauzug` (collectionId/documentId) | `api/src/export_schlauzug.rs:214` | `postgres/src/export_schlauzug.rs:113,115,120,138` | SQL-scoped |
| POST `/api/v1/reviews`, `/reviews/one` (cardId) | `api/src/quiz.rs:218`, `agent.rs:296` | `postgres/src/library/recall.rs:53-59` (`c.owner_id=$1 AND c.id=$2`) | SQL-scoped |
| PUT/DELETE `/api/v1/recall/devices` (push token) | `api/src/quiz.rs:299,341` | `baukit/baukit-push/src/postgres.rs:563-574` upsert `ON CONFLICT (token) DO UPDATE SET owner_id=EXCLUDED.owner_id`; delete `:546` scoped | By design: last registrant takes the token (needs the device token, which is not enumerable) |
| `/api/v1/integrations/{target}*` | `api/src/links.rs:219,244,264` | keyed by `(owner_id,target)`, `postgres/src/links.rs:239-370` | SQL-scoped (target is an enum, not a resource id) |
| GET/PATCH/DELETE `/api/v1/suite/links/{id}`, `/deliveries`, `/test`, `/replay`, `/reenable` (baukit-suite) | `baukit/baukit-suite/src/services/link.rs:535-580`, `services/delivery.rs:110-130` | `find_link_for_owner` loads `WHERE id=$1` then filters `link.user_id == owner` in Rust (`baukit/baukit-suite/src/adapters/postgres/mod.rs:539-548`). `disconnect` re-checks after `link_for_update` (`link.rs:568-571`). `update_preferences`/`reenable` also add `AND user_id=$2` to the UPDATE (`mod.rs:586-592`, `:624-629`) | Service-check only (latent), writes SQL-scoped |

Side note on suite: `find_link` calls `refresh_failures(id)` before the owner filter (`mod.rs:523-524`, `:1021-1030`), so a GET on a foreign link id runs failure bookkeeping on another user's link. It only folds pending failure jobs into counters, nothing leaks, but it is a write that runs before the owner check.

### 3. Tests

Routes with a resource id (path or body, product-owned, suite excluded): **32** (items GET/PUT/DELETE counted separately, the three export routes separately, devices as one). Routes with a cross-tenant test that uses a second owner and asserts 404/rejection: **23**.

Covered (file:line of the second-owner assertion):
- GET documents/{id}, /content: `crates/lesezeichen-api/tests/library_sync.rs:467,495-500`; content again `:1105-1112`
- GET documents/{id}/file: `crates/lesezeichen-api/tests/uploads.rs:472-473`, `tests/library_sync/pdf.rs:89-92`
- POST uploads/{id}/complete: `tests/uploads.rs:440-441`
- POST documents/{id}/cards/generate: `tests/library_sync/generation.rs:151-162`
- PATCH cards/{id}: `tests/library_sync/cards.rs:180-210`; POST cards with foreign highlight/document: `cards.rs:214-232`
- DELETE words/{id}: `tests/library_sync/words.rs:141-156`
- DELETE feeds/{id}: `tests/feeds.rs:443,514-516`
- DELETE tokens/{id}: `tests/tokens.rs:340-361`
- GET imports/{id} and POST imports with foreign uploadId: `tests/imports/pipeline.rs:384-405`
- GET exports/{id}, /file, /report: `tests/schlauzug_exports.rs:351-359`; POST schlauzug foreign scope via MCP `crates/lesezeichen-mcp/tests/http_contract.rs:537-572`
- GET exports/redemut/{id}: `tests/redemut_push.rs:232-242` (random owner token)
- GET briefings/{id}/audio: `tests/briefing.rs:109,176-187`
- POST reviews with foreign card: `tests/library_sync/cards.rs:518-523`; reviews/one foreign via MCP `http_contract.rs:573`
- POST notes foreign document: MCP `http_contract.rs:543-546`
- POST sync/push foreign row id: repo level `crates/lesezeichen-postgres/tests/library.rs:453-455` ("theft"), `tests/library/cards.rs:88`, `tests/library/recall.rs:144-165` (`id_unavailable`), `tests/library/reading.rs:45`
- GET me/erasures/{operationId}: `tests/identity_erasure_conformance.rs:291-304` via `baukit_test::check_identity_erasure_conformance` with an "other-subject"

Not covered (9): GET/PUT/DELETE `/items/{id}` (only scope tests at `crates/lesezeichen-api/tests/tokens.rs:532-534`), DELETE `/documents/{id}`, GET `/documents/{id}/bundle` over HTTP (MCP `get_document` covers the same service at `http_contract.rs:538`), POST `/documents/{id}/finish`, POST `/words/{id}/retry`, POST `/words` with a foreign documentId at the HTTP level (repo-level parent check test at `postgres/tests/library.rs:1099`), PUT/DELETE `/recall/devices` (only the API-token 403 at `cards.rs:569-582`).

Suite: `tests/suite_sender.rs:931-946` shows another user cannot complete someone else's link request. I found no HTTP test for a foreign `/suite/links/{id}`.

### 4. Rate limiting

Attachment (`crates/lesezeichen-bin/src/bin/api.rs:421-443`):
- `baukit_ratelimit::layers` (identity + IP buckets, defaults 60/min burst 10 per identity and 6000/min per IP, `baukit/baukit-config/src/lib.rs:207-223`) wraps the merged product router built at `api.rs:343-420`. That is every `/items*`, `/me`, `/api/v1/*` product route above.
- `authenticated_route_group` "item_writes", 30 POST/PUT/PATCH/DELETE per minute per subject (`api.rs:32-33,422-433,545-550`), on the same router.
- MCP: `baukit_mcp::router` gets its own store (`api.rs:444-472`). Outside local it refuses to start without Redis.
- Suite signed inbound calls: baukit-suite limits 120/min per link (`baukit/baukit-suite/src/adapters/http.rs:933`), store from `RateLimits::suite` (`crates/lesezeichen-bin/src/rate_limits.rs:21-33`). That store is mandatory outside local.

Gaps:
- The product API limiter is conditional: `if let Some(store) = rate_limit_store` (`api.rs:421`). `RedisRateLimitStore::connect_if_enabled` returns `None` when identity and IP are both disabled (`baukit/baukit-ratelimit/src/redis_store.rs:117-124`). Unlike suite and MCP, production does not fail closed here, so a config with `enabled=false` silently drops all API limits. Fail mode defaults to `Open` (`baukit-config/src/lib.rs:308`).
- Merged after the limiter, so no baukit-ratelimit: interactive suite routes (`/api/v1/suite/links*`, `/suite/authorizations*`, `/suite/peers`, `api.rs:479`); `/platform/inbound` (`api.rs:473`, HMAC-authenticated); `/api/v1/inbound/email/postmark` (`api.rs:480-483`, basic auth, bounded body). The two webhooks are authenticated machine callers, so this is low risk. The interactive suite routes are user-facing and include outbound calls to peers (`/suite/links/{id}/test`, `/replay`, `/links/exchange`). Replay has its own `replay_retry_after` (`baukit-suite/src/services/delivery.rs:131`). Test and link creation have no limiter I could find.
- Abuse-prone routes that only get the generic 60/min identity plus 30/min write bucket, with no dedicated route group: POST `/api/v1/documents` (outbound URL fetch), POST `/feeds` and `/feeds/import-opml` (outbound fetch fan-out), POST `/documents/{id}/cards/generate` and `/finish` (LLM), POST `/words` and `/words/{id}/retry` (LLM enrichment), POST `/briefings` (LLM + TTS), POST `/imports`, `/documents/uploads`, POST `/exports/schlauzug`, `/exports/redemut` (outbound), `/integrations/{target}/start|complete` (OAuth outbound), POST `/tokens`, GET `/documents?q=` (full-text search, bounded by a statement timeout, `postgres/src/library.rs:210`). Several of them carry domain quotas instead: daily word enrichment cap (`postgres/src/words.rs:282`), briefing quota/retry limit (`BriefingError::Quota|RetryLimit`), generation entitlement/credits, and the account export concurrency semaphore (`api/src/account.rs:147`). These cap cost, not request rate.
- No unauthenticated sign-up or session route exists in this backend. Login is external OIDC/Clerk.

### 5. Sync push owner isolation

Isolation holds for every pushed row:
- Reads of the current row are owner-filtered (`read_row`, `postgres/src/library.rs:391`). A foreign id looks absent, so with `base_revision=0` the change goes to insert.
- The insert is `INSERT ... ON CONFLICT (id) DO UPDATE ... WHERE t.owner_id=$2 RETURNING` (`library.rs:498`). A colliding foreign id updates nothing and returns no row. `apply_change` maps that to outcome `id_unavailable` (`library.rs:776-779`), and so does `apply_review` (`library/recall.rs:94-96`). `ON CONFLICT DO UPDATE ... WHERE` false takes no row lock change and writes nothing.
- Parent references (document, source, highlight, collection, tag) are checked with `owner_id=$1` (`library.rs:532-598`). Server-owned tables are refused (`library.rs:637-642`).
- Pull is owner-scoped (`library.rs:1043-1056`).
- Recall settings use the same pattern (`postgres/src/recall/storage.rs:50`, `WHERE recall_settings.owner_id=$1`).
- Tests: `postgres/tests/library.rs:453-455`, `tests/library/recall.rs:144-165`, `tests/library/cards.rs:88`.

One caveat: isolation lives in the `WHERE` clause of an `ON CONFLICT` on a global `id` primary key. A user can probe whether a UUID exists in another tenant (`id_unavailable` vs accepted). With client-generated v4/v7 UUIDs this is not practically exploitable.

### Findings

**Confirmed missing scoping**
- `/items`, `/items/{id}` (GET/PUT/DELETE): template CRUD over a global `items` table without `owner_id`. `crates/lesezeichen-postgres/src/lib.rs:104,119,176,189`; handlers ignore the principal (`crates/lesezeichen-api/src/lib.rs:275-395`). Any interactive session can list, read, rename or delete every user's items. Low data value (scaffold leftover, not used by the product UI as far as I can tell), but reachable in production. Remove the routes or scope the table.

**Service-check only (latent)**
- baukit-suite link reads: `find_link_for_owner` loads by id and filters in Rust (`baukit/baukit-suite/src/adapters/postgres/mod.rs:539-548`), used by GET `/suite/links/{id}`, deliveries, test, replay and disconnect. `refresh_failures` runs a write on the foreign id before the filter (`mod.rs:523-524`). Writes in `update_preferences`/`reenable` also scope in SQL.
- Push device tokens are reassigned to the last registrant by design (`baukit/baukit-push/src/postgres.rs:563-574`).

**SQL-scoped (good)**
- Every other product route with a resource id (29 of 32) reaches SQL that filters `owner_id=$1`, through the shared `read_row`/`write_row` helpers or route-specific queries cited in section 2. Sync push cannot overwrite foreign rows (`library.rs:498`, `id_unavailable`).
- Test coverage: 23 of 32 id-taking routes have a second-owner test. Gaps: items x3, DELETE documents, HTTP bundle, finish, words retry, words capture with a foreign document, recall devices.
- Rate limiting: the product API is covered by identity/IP buckets plus a 30/min write group, but only when Redis is enabled. Production does not enforce that. Interactive suite routes sit outside the limiter, and there are no dedicated buckets for LLM, outbound-fetch or import routes.

---

## eigenruhe

Backend: `/home/patrick/projects/eigenruhe/backend`. Paths below are relative to that directory unless they start with `baukit/` (meaning `/home/patrick/projects/baukit/rust/crates/`). Read-only audit, 2026-11-07.

### 1. Ownership model

- One scope: the product user id (`users.id`, `Uuid`). Every user-owned table has an `owner_id` column. Studio tables use `trainer_id` (same uuid space) and coaching links carry both `trainer_id` and `client_id`. No org or workspace scoping exists.
- Principal: `baukit_auth::Principal`, established by `establish_principal` (layered at `crates/eigenruhe-api/src/lib.rs:523-526`) and taken as a handler argument.
- Principal to owner id: `owner_id(state, principal)` at `crates/eigenruhe-api/src/lib.rs:578-588`. An API token returns `token.owner_id`. Otherwise it calls `state.me.get_or_create(subject, authenticated_at)`, which also does implicit sign-up.
- Every handler in scope calls `owner_id` and passes the uuid down. Every repository port method that takes a resource id also takes `owner_id`/`trainer`/`client` (`crates/eigenruhe-ports/src/lib.rs`, e.g. `get_program` :393, `get` :541, `get_preset` :736, `delete_preset` :749, `get_client` :294). Write methods that take a whole entity (`save_program`, `save_program_slot`, `update_preset`, practice `patch`) carry `owner_id` inside the entity. The SQL upserts add `WHERE <table>.owner_id=EXCLUDED.owner_id`, which blocks overwriting another user's row (`crates/eigenruhe-postgres/src/programs.rs:383,425`, `soundscape.rs:192`, `script_preferences.rs:104`, `sleep_diary.rs:253`). The practice update is `WHERE owner_id=$1 AND id=$2` (`practice.rs:225`).
- The repeated pattern in coaching and studio code is a scoped "gate" query (trainer or client id in the WHERE), then follow-up statements by the ids the gate returned, inside the same transaction. Examples: `locked_client_link` (`coaching.rs:177-198`), `require` (`coaching.rs:962-990`, gate at :971-972), `locked_request` (`coaching_request.rs:104-142`). I count these as SQL-scoped. An unscoped `WHERE id=$1` after the gate (e.g. `coaching.rs:169`, `coaching_plan.rs:57`, `coaching_request.rs:357,460,511`) only runs on an id the gate already proved belongs to the caller.

So E4-05 does not really apply to eigenruhe. Scoping happens in repository SQL, not in service-layer comparisons. Nothing enforces it at the type level, though. The ports take a bare `Uuid` owner.

### 2. Route inventory (authenticated routes with a resource id on tenant-owned tables)

Sources: router `crates/eigenruhe-api/src/lib.rs:214-316` and `:390-504`, studio and coaching routers `crates/eigenruhe-api/src/studio.rs:10-201`, cross-checked against `openapi.json` (99 paths). Out of scope: catalog routes (`content/items/{id}`, `content/scripts/{id}`, `.../animations/{bindingKey}`) are global, non-tenant data, and `public/trainers/{slug}` is public by design.

#### General user data

| Route | Handler | Repo SQL (scoping) | Class | Cross-tenant test |
|---|---|---|---|---|
| GET `/programs/{id}` | `program_handlers.rs:162` | `programs.rs:526` (`id AND owner_id`), slots `:538` | SQL-scoped | none |
| PATCH `/programs/{id}` | `program_handlers.rs:195` | load `programs.rs:526`, lock `:180`, upsert guard `:383` | SQL-scoped | none |
| POST `/programs/{id}/slots/{slotId}/skip\|move\|swap` (3) | `program_handlers.rs:228,254,287` | program via `get_program(owner,id)`. Slot picked from that program's owner-scoped slots (`eigenruhe-services/src/programs.rs:519-523`), lock `programs.rs:228`, upsert guard `:425` | SQL-scoped | none |
| POST `/programs/{id}/adaptation/preview\|apply` (2) | `program_adaptation_handlers.rs:26,56` | `program_adaptation.rs:165,498,590,691` (all `owner_id`) | SQL-scoped | none |
| GET `/soundscape-presets/{id}` | `soundscape_handlers.rs:84` | `soundscape.rs:87` | SQL-scoped | MCP `tests/mcp_conformance.rs:207-215` (404) |
| PATCH `/soundscape-presets/{id}` | `soundscape_handlers.rs:119` | `soundscape.rs:349`, upsert guard `:192` | SQL-scoped | none |
| DELETE `/soundscape-presets/{id}` | `soundscape_handlers.rs:157` | `soundscape.rs:397,411` | SQL-scoped | none |
| GET `/practice-sessions/{id}` | `practice_handlers.rs:97` | `practice.rs:394` | SQL-scoped | MCP `tests/mcp_conformance.rs:266-273` (404) |
| PATCH `/practice-sessions/{id}` | `practice_handlers.rs:130` | `practice.rs:168` (lock), `:225` | SQL-scoped | none |
| POST `/practice-sessions/{id}/complete` | `practice_handlers.rs:168` | session get `practice.rs:394`, update `:225`, check-in upsert guard `:261-266` | SQL-scoped | none |
| POST `/practice-sessions/{id}/check-in` | `practice_handlers.rs:207` | service `sessions.get(owner,id)` (`eigenruhe-services/src/practice.rs:337`), then `practice.rs:300,313` | SQL-scoped | none |
| PUT `/practice-sessions/{id}/tags` | `practice_handlers.rs:241` | `eigenruhe-services/src/practice.rs:355-359`, `practice.rs:554` (`owner_id` arg) | SQL-scoped | none |
| PUT\|DELETE `/sleep-diary/{localDate}` (2) | `sleep_handlers.rs:51,94` | keyed by `(owner_id, local_date)`: `sleep_diary.rs:78,109`. Write path `write_entry` `:187-199` loads by id and compares owner in Rust (see latent list) | SQL-scoped (key is owner+date) | DELETE: `tests/sleep_api.rs:153-160` (404). PUT: none, and no foreign id is reachable |
| PUT `/script-preferences/{track}` | `script_preferences_handlers.rs:50` | `script_preferences.rs:45`, upsert guard `:104` | SQL-scoped (owner+track) | n/a |
| DELETE `/api-tokens/{id}` | `api_token_handlers.rs:91` | `baukit/baukit-auth/src/postgres.rs:138-139` (`owner_id AND id`) | SQL-scoped | `tests/auth_conformance.rs:410-413` (404) |
| GET `/operations/{id}` | `connection_handlers.rs:164` | `operations.rs:20-22` (`payload->>'owner_id'=$3`) | SQL-scoped | repo `tests/postgres_integration.rs:3086-3091`, unit `connection_handlers.rs:253` |
| GET `/me/erasures/{operationId}` | `me_handlers.rs:120` | `baukit/baukit-erasure/src/store.rs:217` (`id AND subject_hash`) | SQL-scoped | `tests/identity_erasure_conformance.rs:299-316` (404) |
| DELETE `/connections/{provider}`, POST `.../sync-now` (2) | `connection_handlers.rs:80,123` | `connections.rs:162,233,251` (`owner_id AND provider`) | SQL-scoped (owner+provider) | n/a |

#### Coaching (client side, `/api/v1/coaching`)

| Route | Handler | Repo SQL | Class | Cross-tenant test |
|---|---|---|---|---|
| DELETE `/requests/{id}` | `coaching_request_handlers.rs:72` | `coaching_request.rs:112,140` (`r.id AND r.client_id`) | SQL-scoped | none |
| POST `/requests/{id}/accept` | `coaching_request_handlers.rs:111` | same gate | SQL-scoped | `tests/connect_requests_api.rs:204-219` (404) |
| POST `/requests/{id}/decline` | `coaching_request_handlers.rs:152` | same gate | SQL-scoped | none |
| POST `/offers/{id}/accept` | `coaching_plan_handlers.rs:270` | gate `coaching_plan.rs:369-372` (`o.id AND l.client_id`), then `load` :57 | SQL-scoped | `tests/coaching_api.rs:2647-2664` (404) |
| POST `/offers/{id}/decline` | `coaching_plan_handlers.rs:333` | same gate | SQL-scoped | none |
| PATCH `/links/{id}`, POST `/links/{id}/revoke` (2) | `coaching_handlers.rs:317,350` | `locked_client_link` `coaching.rs:183,195` (`id AND client_id`) | SQL-scoped | `tests/coaching_api.rs:391-401` (404) |
| GET `/links/{id}/access-log` | `coaching_handlers.rs:381` | gate `coaching.rs:801-802`, then rows :811 | SQL-scoped | `tests/coaching_api.rs:391-401` |

Token and code routes (`GET /invitations/{token}`, `POST /invitations/{token}/accept|decline`, `POST /invitations/redeem-code`, `POST /invitations/by-code/accept|decline`, 6 routes) are bearer-credential lookups by hash (`coaching.rs:238,908`, `coaching/group_invitation.rs:165,203,250`). Knowing the token is the authorization, by design. They are rate limited (section 4). Tests: `tests/coaching_api.rs:504-530` (wrong credentials), `:2117` (code accept by another user).

#### Studio (trainer side, `/api/v1/studio`, also gated by `require_trainer_plan` `studio.rs:260-282`)

| Route | Handler | Repo SQL | Class | Cross-tenant test |
|---|---|---|---|---|
| POST `/requests/{id}/accept\|decline` (2) | `coaching_request_handlers.rs:228,273` | `coaching_request.rs:112,140` (`r.trainer_id`) | SQL-scoped | none |
| GET\|PATCH\|DELETE `/templates/{id}` (3) | `studio_template_handlers.rs:141,200,241` | `studio_template.rs:120,150,165,196` (`trainer_id AND id`) | SQL-scoped | `tests/studio_api.rs:2515-2530` (404) |
| GET\|POST `/groups/{id}/assignments`, GET\|DELETE `/groups/{id}/assignments/{aid}` (4) | `studio_group_assignment_handlers.rs:157,87,223,286` | `studio_group_assignment.rs:250,260,332` | SQL-scoped | `tests/studio_api.rs:3584-3600` (404) |
| GET\|POST `/groups/{id}/invitations` (2) | `group_invitation_handlers.rs:85,46` | `coaching/group_invitation.rs:31,107` | SQL-scoped | `tests/coaching_api.rs:2225-2236` |
| DELETE `/group-invitations/{id}` | `group_invitation_handlers.rs:119` | `coaching/group_invitation.rs:141` (join `g.trainer_id`) | SQL-scoped | `tests/coaching_api.rs:2247` (404) |
| GET\|POST `/clients/{id}/offers`, DELETE `/offers/{id}`, GET `/clients/{id}/programs`, PATCH `/clients/{id}/programs/{pid}/slots/{sid}` (5) | `coaching_plan_handlers.rs:44,131,199,392,496` | gate `require` `coaching.rs:971`. Withdraw `coaching_plan.rs:446-447`. Body `templateId` scoped `coaching_plan.rs:245`. Slot edit: program `WHERE id AND owner_id=<client from active link>` `coaching_plan.rs:595-596`, then `author == link` check :605 | SQL-scoped | matrix `tests/coaching_api.rs:3054-3160` (outsider, missing scope, revoked link) |
| GET\|PATCH `/groups/{id}`, GET `/groups/{id}/overview` (3) | `studio_group_handlers.rs:130,222,175` | `studio_group.rs:141,277,325` | SQL-scoped | `tests/studio_api.rs:995-1010`, `tests/coaching_api.rs:3775-3785` |
| PUT\|DELETE `/groups/{id}/members/{clientId}` (2) | `studio_group_handlers.rs:273,317` | group gate `studio_group.rs:183` (`locked_group`), client gate `locked_client` :184, member `:194,241` | SQL-scoped | `tests/studio_api.rs:1015-1055` |
| GET\|PATCH `/clients/{id}` (2) | `studio_handlers.rs:298,330` | `studio.rs:295,323,352` | SQL-scoped | `tests/studio_api.rs:362-372` (404) |
| GET `/clients/{id}/overview\|activity\|tension` (3) | `studio_tracking_handlers.rs:36,69,120` | gate `coaching.rs:971` via `studio_tracking.rs:92-101`. Follow-ups by the gated `link_id` `studio_tracking.rs:226,285` | SQL-scoped | `tests/coaching_api.rs:1695-1710` |
| GET\|POST `/clients/{id}/attendance`, DELETE `/clients/{id}/attendance/{attendanceId}` (3) | `studio_handlers.rs:446,398,495` | `studio_attendance.rs:31,53,86,119` | SQL-scoped | `tests/studio_api.rs:1593-1620` (404) |
| POST `/clients/{id}/invitations` | `studio_handlers.rs:607` | `coaching.rs:258` (`id AND trainer_id`) | SQL-scoped | `tests/coaching_api.rs:976-990` |
| DELETE `/invitations/{id}` | `studio_handlers.rs:652` | `coaching.rs:918-921` (join `c.trainer_id`) | SQL-scoped | `tests/coaching_api.rs:589` (404) |
| POST `/clients/{id}/end` | `studio_handlers.rs:683` | `coaching.rs:490-494` (`c.trainer_id AND l.trainer_id`) | SQL-scoped | `tests/coaching_api.rs:783-808` via :832 |

`/studio/profile*` and `/studio/verification` take no resource id (owner-keyed, `studio.rs:69,111,206`, `trainer_verification.rs:30,100`). The by-id trainer verification statements (`trainer_verification.rs:138,164,188`) only serve the operator CLI decide path. No HTTP route reaches them.

#### Suite (`/api/v1/suite/*`, baukit-suite tables, mounted by `crates/eigenruhe-api/src/suite.rs:19-28`)

These are baukit code, listed for completeness. GET `/links/{id}`, `/links/{id}/deliveries`, POST `/links/{id}/replay|test` and DELETE `/links/{id}` go through `find_link_for_owner`. That function loads by id and filters in Rust (`baukit/baukit-suite/src/adapters/postgres/mod.rs:523-547`). DELETE also re-checks `link.user_id != owner` after `link_for_update` (`baukit/baukit-suite/src/services/link.rs:566-571`). PATCH `/links/{id}` and POST `/links/{id}/reenable` scope in SQL (`mod.rs` `WHERE id=$1 AND user_id=$2`, :589-590 and :626-627). `/links/callback`, `/links/exchange`, `/inbound/{linkId}`, `/links/{remoteLinkId}/revoke` skip principal auth (`suite.rs:36-41`) and use signed peer credentials instead.

**Totals (eigenruhe-owned routes):** 64 route-method pairs take a resource id on tenant-owned tables (23 general, 9 coaching client-side, 32 studio), plus 6 token/code bearer routes and 9 suite link routes. SQL-scoped: 64 of 64. Service-check-only: 0. MISSING: 0.

### 3. Tests

I searched by behavior: tests that mint a second subject (`authorization(&issuer, "<x>-other|outsider|stranger"...)`, `claims("other-sleep-user"|"another-subject"|"second-account")`) and assert 404 on another user's id. The extraction script scanned every `&other`/`&outsider`/`&stranger` call site in `tests/*.rs`.

- **42 of 64** routes have a cross-tenant test: 36 at the HTTP level, 2 through MCP tools on the same services (soundscape GET, practice GET), 1 at repo level (operations), plus api-token revoke, erasure status and sleep-diary DELETE at the HTTP level. Main files: `tests/studio_api.rs`, `tests/coaching_api.rs`, `tests/connect_requests_api.rs`, `tests/auth_conformance.rs:410`, `tests/identity_erasure_conformance.rs:299`, `tests/sleep_api.rs:153`, `tests/mcp_conformance.rs:207,266`, `tests/postgres_integration.rs:3086`.
- **Untested (22):** programs GET/PATCH, slot skip/move/swap, adaptation preview/apply (7). Soundscape PATCH/DELETE (2). Practice PATCH/complete/check-in/tags (4). Sleep PUT, script-preferences PUT, connections DELETE and sync-now (4; owner-keyed, no foreign id is expressible). Coaching request withdraw and decline (2). Offer decline (1). Studio request accept/decline (2). The program, soundscape and practice routes are the real gap: they take a client-supplied uuid and no test runs them with a second user.
- Repo-level coverage on top of that: `crates/eigenruhe-postgres/tests/coaching.rs` (`list_access_log(second_owner, ...)` :2419), `studio_attendance.rs:416`, `studio_group_repository.rs:326,674`, `coaching/requests.rs`.
- Sync: the only cross-owner push test is for sleep-diary entries (`tests/postgres_integration.rs:3891-3892`, `invalid_owner`). No test pushes a foreign id for programs, slots, sessions, check-ins, tags, presets or script preferences.

### 4. Rate limiting

All layers are baukit-ratelimit, built in `crates/eigenruhe-api/src/lib.rs`. Defaults are in `crates/eigenruhe-api/src/rate_limit.rs:56-77` (per minute: write 60, compute 30, sensitive 10, sync push 30 / pull 60, animation descriptors 60, plus coaching-invitation identity and IP scopes).

- Global identity limiter wraps every authenticated route: `lib.rs:518`. Global IP limiter wraps the authenticated router, the public trainer route and openapi: `lib.rs:538-542`.
- Route groups (`authenticated_route_group`, keyed by `principal.subject()`):
  - `write`: POST/PATCH/PUT/DELETE on general, studio and coaching routes, `lib.rs:374-385`.
  - `sync_push` (`:391-402`) and `sync_pull` (`:405-416`).
  - `sensitive`: device tokens, fitness connect/disconnect, sync-now, operations, API-token create, `:418-453`. Erasure routes also use the `sensitive` group, `:486-504`.
  - `compute`: program preview, adaptation preview/apply, insights, sleep overview, `:455-484`.
  - `animation_descriptors`, `:317-334`.
  - `coaching_invitations`: identity at `:339-355` and IP at `:356-368`, covering connect-request create, invitation preview/accept/decline, code redeem.
- Public `/public/trainers/{slug}`: IP limiter only, tested at `tests/studio_api.rs:2342`.
- MCP: `baukit_mcp::router` gets the shared store (`crates/eigenruhe-bin/src/bin/api.rs:421-427`) plus `eigenruhe_mcp::ServiceQuota` (:406-411).

Not covered by any router-level limiter:

- **All `/api/v1/suite/*` routes and `/platform/inbound`.** `extend` merges `suite_routes` (and `mcp`) after the IP layer has been applied (`lib.rs:538` with `api.rs:445`), and `platform.router` is merged after that (`api.rs:447`, `baukit/baukit-suite/src/platform/receiver.rs:68-70`). Axum layers only wrap routes that exist when `.layer` is called, so these routes skip both the identity and IP limiters. baukit-suite limits only inbound events per link (`baukit/baukit-suite/src/services/ingest.rs:143,151`) and exchange failures (`services/link.rs:377`). Link start, test, replay and reenable trigger outbound calls to peer apps with no limit, and unauthenticated callback, exchange and revoke get no IP limit.
- Studio invitation creation (`POST /studio/clients/{id}/invitations`, `POST /studio/groups/{id}/invitations`) and offer creation only get the generic `write` group (60/min), not the `coaching_invitations` scope. It is a trainer-only, plan-gated feature, so the risk is limited.
- Reads such as `GET /content/items?q=` (ILIKE search, `content.rs:34`) and studio tracking reads only get the global identity and IP limits. Nothing per route.
- Auth/session and sign-up: eigenruhe has no login endpoints (OIDC is external). Implicit sign-up through `get_or_create` runs on any authenticated request under the global limits. The project has no export, upload or LLM routes.

### 5. Sync (baukit-sync push/pull)

Used: `POST /sync/push`, `GET /sync/pull` (`lib.rs:390-416`), repo `crates/eigenruhe-postgres/src/sync.rs`.

- Push locks the owner's revision row (`sync.rs:52-60`). For every change, `apply_change` runs `id_is_owned_or_missing` (`sync.rs:376-381`, function :918-931). It selects `owner_id` by id and rejects with `invalid_owner` when the row belongs to someone else. This is a Rust comparison without `FOR UPDATE`.
- Second line of defense: every per-entity upsert has `ON CONFLICT (id) DO UPDATE ... WHERE <table>.owner_id=EXCLUDED.owner_id` (questionnaires :580, programs :633, program_slots :772, practice_sessions :1009, check_ins :1094, tags :1189, presets `soundscape.rs:192`, script preferences `script_preferences.rs:104`, sleep diary `sleep_diary.rs:253`). User settings are keyed by `owner_id` (`settings.rs:40`). External-connection disable is SQL-scoped (`sync.rs:519-520,545`). Sleep diary also re-checks under `FOR UPDATE` (`sleep_diary.rs:187-199`).
- References are owner-checked too: a slot's `program_id` (`sync.rs:664-665` `program_weeks(owner,...)`) and a check-in's or tag's `practice_session_id` (`session_exists(owner,...)` :1047, :1123).
- Result: a push cannot overwrite another user's row. One small race remains. If two users push the same new id at the same moment, the loser's upsert turns into a no-op through the guard, but `apply_change` still reports `Accepted` with a fresh revision. I see no data leak or corruption, only a misleading result for the loser.
- Pull is scoped by `owner_id` throughout (`sync.rs:165-210`).

### Findings

**Confirmed missing scoping**
- None in eigenruhe. All 64 resource-id routes scope in SQL, either directly or through a scoped gate query in the same transaction.

**Service-check only (latent)**
- `crates/eigenruhe-postgres/src/sync.rs:918-931` `id_is_owned_or_missing` reads without a lock and compares in Rust. The upsert `WHERE owner_id=EXCLUDED.owner_id` guards cover it, but a concurrent first insert of the same id by two owners returns `Accepted` to the losing owner while nothing was written.
- `crates/eigenruhe-postgres/src/sleep_diary.rs:187-199` `write_entry` loads by id and compares `owner_id` in Rust. It is locked (`FOR UPDATE`) and safe today. Only sync push and the owner+date PUT path reach it.
- baukit-suite `find_link_for_owner` (`baukit/baukit-suite/src/adapters/postgres/mod.rs:539-547`) loads by id, then filters `user_id` in Rust. It backs suite GET, deliveries, replay, test and delete. This is baukit code, not eigenruhe.
- Not a code issue: 22 of 64 id routes have no cross-tenant test. The general programs, soundscape PATCH/DELETE and practice write routes are the ones that matter. Sync push has a cross-owner test for sleep-diary entries only.

**SQL-scoped (good)**
- Programs, slots, adaptation, soundscape, practice, check-ins, tags, sleep diary, script preferences, connections, operations (`payload->>'owner_id'`), API tokens (baukit-auth), and erasure status (baukit-erasure `subject_hash`).
- All coaching and studio routes: the trainer/client gate queries are at `coaching.rs:183,801,918,971,490`, `coaching_request.rs:112,140`, `coaching_plan.rs:245,372,447,595` and `coaching/group_invitation.rs:107,141`, and `studio*.rs` put `trainer_id` in every WHERE.

**Rate limiting gap (for section 4 of the plan)**
- `/api/v1/suite/*` (including unauthenticated callback, exchange and revoke) and `/platform/inbound` are merged after the identity and IP layers (`crates/eigenruhe-api/src/lib.rs:538`, `crates/eigenruhe-bin/src/bin/api.rs:445,447`), so router-level rate limiting never applies to them. baukit-suite only limits inbound events per link and exchange failures.

---

## schluesselbund

Paths below are relative to `/home/patrick/projects/schluesselbund/backend/`. `pg/` = `crates/schluesselbund-postgres/src/`, `svc/` = `crates/schluesselbund-services/src/`, `api/` = `crates/schluesselbund-api/src/`, `bin/` = `crates/schluesselbund-bin/src/`.

### 1. Ownership model

Three tenant scopes, plus a product scope for the platform API:

- **Account** (`AccountId(pub Uuid)`, `crates/schluesselbund-domain/src/account.rs:11`). Personal data (subscriptions, tokens, Kompass conversations/actions/connections/weekly plans, push devices, credits, onboarding) carries `account_id` / `owner_id`.
- **Organization** (`OrganizationId(pub Uuid)`, `crates/schluesselbund-domain/src/organization.rs:11`). Membership in `organization_members(organization_id, account_id, account_role, status)`. Roles owner/admin/billing/member gate actions through `Permission`.
- **Family** (`families.id: Uuid`), membership in `family_members`, one organizer role.
- **Platform caller** (product service token). Rows are filtered by `caller.product_id()` and by `account_products`.

How the principal reaches a handler:
- `baukit_auth::Principal` comes from `baukit_auth::establish_principal`, attached at `bin/bin/api.rs:475-478`. Handlers take `principal: Principal` as an extractor.
- `api/routes/me.rs:100` (`identity`) turns it into a `HubIdentity`. `AccountService::current` (`svc/accounts.rs:91`) resolves that to an `Account`. Some routes use `account_subject` (`api/lib.rs:276`) instead.
- Org repositories wrap the account in `OrganizationActor { account, authenticated_at, line }` (`crates/schluesselbund-ports/src/organizations.rs:60`). Families use `FamilyActor`.
- Organization API tokens are pinned to their org in middleware. `api/token_access.rs:41-58` compares path `{id}` with `token.owner_id` and allows only `GET members` and `GET audit`. Personal API tokens are denied everywhere except `/v1/me`, `/v1/me/apps`, `/v1/me/entitlements` and the admin routes (`api/token_access.rs:60-90`).

No typed tenant wrapper exists. Every org/family route goes through a central guard instead of a scoped query:
- **Organizations:** `authorize()` at `pg/organizations.rs:187-221`. It loads the org by id, then reads the actor's membership with `WHERE organization_id=$1 AND account_id=$2` (`pg/organizations.rs:174`), then checks the role in Rust (`actor_member.authorize(permission)`). The data queries that follow filter by `organization_id` only.
- **Business billing** has its own guard, `pg/business_billing.rs:26-64` / `66-93`. It is an SQL `EXISTS` on membership plus role `IN ('owner','admin')`.
- **Families:** `lock` + `load` by id (`pg/families.rs:129-134`), then the code looks for `me` in the loaded member list and runs `organizer(me)` (`pg/families.rs:564-570`).

### 2. Route inventory (authenticated routes taking a resource id)

These classes are adapted from the brief:
- **SQL-scoped.** The owner or tenant is in the WHERE clause of the query that reads the target row.
- **Guard (SQL membership).** A separate SQL query checks membership and role for the actor, then data queries filter by org id. A missed guard call would leak, so this is latent like service-check, but the check itself runs in SQL.
- **Service-check.** The code loads the row by id and compares in Rust.

#### Items (template leftover)

| Route | Handler | Repo SQL | Class |
|---|---|---|---|
| GET/PUT/DELETE `/items/{id}` (also GET/POST `/items`) | `api/lib.rs:365`, `:419`, `:446` (the `_principal` argument is ignored) | `pg/lib.rs:106` `SELECT ... FROM items WHERE id=$1`, `:163` UPDATE, `:176` DELETE | **MISSING** |

`migrations/0001_create_items.sql` has no owner column, so every authenticated hub user can read, rename and delete every item, and `GET /items` lists them all (`api/lib.rs:338-357`). The table looks like an unused scaffold, but it is live and in `openapi.json`.

#### Organizations (all under `api/lib.rs:119-133`)

| Route(s) | Handler | Check | Data query | Class |
|---|---|---|---|---|
| GET/PATCH `/v1/organizations/{id}`, GET `/{id}/members`, PATCH/DELETE `/{id}/members/{accountId}`, POST `/{id}/leave`, POST `/{id}/transfer` | `api/routes/organizations.rs:314-540` → `svc/organizations.rs:40,173` | `authorize()` at `pg/organizations.rs:535-575`, `:878-889`, `:936`. Target member loaded with `member(conn, actor.organization_id, account_id)` (`pg/organizations.rs:483`, `:507`, `:551`) | members `pg/organizations.rs:938` `WHERE organization_id=$1` | Guard (SQL membership) |
| GET/POST `/{id}/invitations`, DELETE `/{id}/invitations/{invitationId}`, POST `.../resend` | `api/routes/invitations.rs` | `authorize()` at `pg/invitations.rs:399`, `:440`, `:484`, `:698` | invitation row `pg/invitations.rs:266-267` `WHERE organization_id=$1 AND id=$2` | Guard, plus child row SQL-scoped |
| GET/PUT `/{id}/seats/{productId}` | `api/routes/seats.rs` | `pg/seats.rs:160`, `:226` | `set_seats` takes the requested accounts only from active members of that org (`pg/seats.rs:233-234`) | Guard |
| GET `/{id}/products`, PUT `/{id}/products/{productId}` | `api/routes/organization_products.rs` | `pg/organization_products.rs:190`, `:251` | by org id | Guard |
| POST `/{id}/directory/portal-links`, GET `/{id}/directory-groups`, PUT `/{id}/directory-groups/{groupId}/products` | `api/routes/directory.rs` (`work_only` layer `api/lib.rs:126-131`) | `pg/directory.rs:685`, `:710`, `:733`, `:755`, `:780` | `group_row` `pg/directory.rs:121` `WHERE g.organization_id=$1 AND g.directory_group_id=$2` | Guard, plus child row SQL-scoped |
| GET/POST/DELETE `/{id}/tokens` (DELETE takes `tokenId` in the body), GET `/{id}/audit` | `api/routes/organization_access.rs:117-268` | `pg/organization_access.rs:155`, `:190`, `:250`, `:317`. Token readers go through `authorize_token` `:108` (`WHERE id=$1 AND organization_id=$2`) | revoke `pg/organization_access.rs:261` `WHERE organization_id=$1 AND id=$2`. Audit `:346` `scope_id=$1` | Guard, plus child row SQL-scoped |
| POST `/{id}/checkout`, GET `/{id}/subscriptions`, PUT `/{id}/subscriptions/{subscriptionId}`, GET `/{id}/invoices`, GET `/{id}/invoices/{invoiceId}/pdf` | `api/routes/business_billing.rs:25-35` → `svc/business_billing.rs` | `pg/business_billing.rs:26-64` (EXISTS membership + owner/admin, in SQL) at `:623`, `:673`; `authorize_read` at `:991` and for subscriptions at `:554` | subscription `pg/business_billing.rs:676-680` `WHERE id=$1 AND owner_kind='organization' AND owner_id=$2`. Invoice PDF: Paddle transaction's `subscription_id` must be in the org's list (`crates/schluesselbund-billing/src/paddle.rs:793-798`) | SQL-scoped (PDF: provider-side check against the SQL-scoped list) |

`authorize` calls `require_world` on an existing org before it checks membership (`pg/organizations.rs:205-207`). An outsider on the other identity line gets `account_world_mismatch` for a real org and `organization_forbidden` for a missing one, so the two responses tell them whether an org id exists. This is minor because ids are UUIDs.

#### Families (`api/routes/families.rs:26-47`, consumer line only via `api/lib.rs:111-115`)

| Route(s) | Handler | Check | Class |
|---|---|---|---|
| POST `/{id}/leave`, DELETE `/{id}/members/{accountId}`, PUT `/{id}/organizer`, PUT `/{id}/members/me/visibility`, PUT `/{id}/name`, POST `/{id}/invitations`, DELETE `/{id}/invitations/{invitationId}`, POST `.../resend` | `api/routes/families.rs:259-589` → `svc/families.rs:51` → `pg/families.rs:888` → `apply` | lock + load by id `pg/families.rs:129-134`; `me` from the loaded list; `organizer(me)` at `:192`, `:277`, `:368`, `:396`, `:401`; `me.ok_or(...)` at `:252`, `:304`, `:342`; target member `active_member(&before, ...)` `:571-577`. Invitation child row `pending_row` `pg/families.rs:540` `WHERE family_id=$1 AND id=$2` | Service-check (latent) |
| GET `/{id}/invitations`, GET `/{id}/audit` | `pg/families.rs:914-927`, `:950-978` | load family, then `organizer(...)` in Rust; data `WHERE family_id=$1` / `scope_id=$1` | Service-check (latent) |
| POST `/v1/checkout/sessions` with `familyId` in the body | `api/routes/billing.rs` → `svc/billing.rs:422` | `pg/billing.rs:393-399` loads the family by id and then compares `organizer != Some(account_id)` | Service-check (latent) |

Every family write runs inside `mutate`, which goes through one `apply` with a single `me` lookup, so the check is consistent. Still, nothing in SQL ties `family_id` to the caller. The load happens before the check, so a non-member gets `family_forbidden` (organizer ops) or 404 (leave, visibility) depending on the route, which tells them the family exists.

#### Account-owned resources

| Route | Repo SQL | Class |
|---|---|---|
| POST `/v1/me/subscriptions/{id}/plan-change` | `svc/plan_change.rs:76-82` searches the account's held list; `pg/billing.rs:1175` `WHERE s.owner_kind='account' AND s.owner_id=$1` | SQL-scoped |
| POST `/v1/kompass/actions/{id}/approve`, `/dismiss` | `pg/actions.rs:85-94` `WHERE a.id=$1 AND a.account_id=$2 ... FOR UPDATE OF a`. The follow-up UPDATEs at `pg/actions.rs:176-180` and `:280` filter by `id` only, but they run inside the same transaction after the locked scoped read. | SQL-scoped |
| POST `/v1/kompass/conversations/{id}/messages` | `pg/conversations.rs:291` `WHERE id=$1 AND account_id=$2 AND expires_at>now() FOR UPDATE` | SQL-scoped |
| DELETE `/v1/me/push-devices/{deviceId}`; PUT `/v1/me/push-devices` with `deviceId` in the body | `pg/weekly.rs:635` and `:582` `WHERE id=$1 AND account_id=$2` | SQL-scoped |
| GET `/v1/me/erasure/{id}` | `pg/erasure.rs:310` `WHERE id=$1 AND subject_hash=$2` | SQL-scoped |
| DELETE `/v1/me/tokens` with `tokenId` in the body | `pg/tokens.rs:302` `WHERE owner_id=$1 AND id=$2` | SQL-scoped |
| GET `/v1/credits/{wallet}/entries` (wallet is a catalog key) | `pg/credits.rs:1601` `WHERE account_id=$1 AND (... wallet=$2)` | SQL-scoped |
| POST/PATCH/DELETE `/v1/kompass/connections/{productId}` | `pg/connections.rs:318`, `:371`, `:400` `WHERE account_id=$1 AND product_id=$2` | SQL-scoped |
| GET `/v1/kompass/weekly-plans/{weekStart}` | `pg/weekly.rs:278` `WHERE account_id=$1 AND week_start=$2` | SQL-scoped |
| PUT `/v1/me/onboarding/shares/{productId}`, POST `/v1/me/apps/{productId}/opened` | `pg/onboarding_shares.rs:144`, `:167` keyed on `(account_id, product_id)`; `svc/catalog.rs:128-130` | SQL-scoped |

The push-token PUT reassigns a raw FCM/APNs token to the most recent registrant: `pg/weekly.rs:611` deletes the other account's mapping. That is ordinary bearer-token semantics for push, not a tenancy hole, as long as tokens stay secret.

#### Operator and admin (no tenant)

`DELETE /v1/admin/entitlement-grants/{grantId}` and the four `/v1/admin/product-receivers/{productId}*` routes are global operator routes. They require the `admin:entitlements` or platform-admin grant (`api/routes/entitlements.rs:204-227`, `api/routes/receivers.rs:106-122`). Users cannot mint either grant: `TokenGrantRequirement::ALL` contains only `account:read` (`crates/schluesselbund-domain/src/tokens.rs:36`).

#### Platform API (`/platform/v1/*`, product-scoped, `api/routes/platform.rs`)

| Route(s) | Scoping | Class |
|---|---|---|
| `/accounts/{suiteSubject}`, `/credits/{suiteSubject}`, `/entitlements/{suiteSubject}`, `/onboarding/{suiteSubject}`, `/preferences/{suiteSubject}`, POST `/entitlements/tickets`, `/accounts/{s}/family`, `/accounts/{s}/organizations` | `visible_account` → `pg/accounts.rs:408-411` `JOIN account_products p ... WHERE a.suite_subject=$1 AND p.product_id=$2`; `require_product` `svc/platform.rs:339` | SQL-scoped (by product) |
| POST `/credits/reservations/{id}/settle`, `/release` | `pg/credits.rs:1403` `WHERE id=$1 AND wallet=$2`, where wallet = caller product | SQL-scoped |
| POST `/erasures/{erasureId}/receipts` | `pg/erasure.rs:342` `WHERE erasure_id=$1 AND product_id=$2` | SQL-scoped |
| GET `/organizations/{id}`, `/organizations/{id}/members` | `pg/seats.rs:609-636` (product must be visible for the org); `pg/organization_products.rs:284` `WHERE op.organization_id=$1 AND op.product_id=$2`; caller product equality at `svc/platform.rs:174` | SQL-scoped |
| GET `/families/{familyId}/members` | `pg/families.rs:1072-1100` loads any family by id, then filters members in Rust with `family::visible` (`crates/schluesselbund-domain/src/family.rs:161-169`: `used_products` contains the product and the product is not hidden) | Service-check (latent). Any product holding `families:read` can probe any family id, and only the in-memory filter hides members. |

### 3. Tests (cross-tenant behavior)

**Totals.** Of the 49 human-facing routes that take an explicit resource id (items 3, org 27, family 11, account 8), 26 have a test where a second user or org gets 403/404:
- Org: 18 of 27.
- Account: 7 of 8. The gap is plan-change.
- Family: 2 of 11.
- Items: 0 of 3.

The 6 account-implicit key routes (product or week keys) have no cross-tenant tests, but there is nothing cross-tenant to address in them.

Cross-tenant tests that exist:
- **Org core, 7 routes.** `tests/organizations.rs:344-362` runs every org route in the role matrix as `outsider` and asserts 403 `organization_forbidden`.
- **Org invitations.** Outsider list gets 403 (`tests/invitations.rs:674-679`). DELETE of an invitation through a different org id gets 404 (`tests/invitations.rs:663-673`). Create and resend are not covered by outsider tests.
- **Seats GET/PUT.** Outsider gets 403 (`tests/seats.rs:330-351`). Seat labels are denied to outsiders (`tests/seats.rs:1207-1254`).
- **Directory.** Outsider portal-link gets 403 (`tests/directory_sync.rs:576-581`). Groups GET/PUT have no outsider test.
- **Org tokens and audit.** The outsider matrix covers tokens GET/POST/DELETE and audit (`tests/support/organization_access.rs:164-200`). A foreign org's DELETE gets 404 (`:535-547`). A foreign-org audit cursor is rejected (`:963-972`).
- **Business billing.** An invoice PDF for a subscription the org doesn't own returns `Forbidden`, at service level (`tests/billing.rs:2805-2815`). Checkout, subscriptions and invoices are tested only against same-org `member`/`billing` roles (`tests/billing.rs:2664-2690`, `:3754-3792`). No test uses an outsider or a foreign `subscriptionId`.
- **Families.** A former member is denied audit with 403 (`tests/families.rs:2241-2244`). An outsider's checkout with `familyId` gets `FamilyForbidden` (`tests/support/family_plan.rs:330-342`). Other family routes are tested only with a non-organizer member (`tests/families.rs:1423-1434`, `:1859-1862`), never with a non-member.
- **Account routes.**
  - Foreign conversation message: 404 (`tests/conversations.rs:1989-1999`).
  - Foreign action approve/dismiss: 404 (`tests/conversations.rs:2543-2547`).
  - Foreign push-device DELETE and PUT with `deviceId`: 404 (`tests/weekly.rs:762-800`).
  - Foreign token revoke: 404 (`tests/developer.rs:566-580`).
  - Foreign erasure status: `NotFound`, at service level (`tests/erasure.rs:633`).
  - Plan-change is tested only with a random UUID (`tests/support/tier_changes.rs:368-370`), never with another user's subscription.
- **Platform.** Cross-product tests cover:
  - foreign product on an org summary: 404 (`tests/platform.rs:1968`)
  - foreign product on reservation settle/release: 404 (`tests/platform.rs:2418-2435`)
  - hidden or unknown preferences subjects: 404 (`tests/platform.rs:1520-1530`)
  - onboarding (`tests/platform.rs:1415`)
  - business members (`tests/platform.rs:2054-2208`)
  - erasure receipt product mismatch (`tests/erasure.rs:390-401`)
- **Items.** No test.

### 4. Rate limiting

- **Global layer.** `baukit_ratelimit::layers(api, store, rate_limit_options)` is attached at `bin/bin/api.rs:469`. It covers every route in `routes_with_authenticated` and the merged developer, organization-access and credits routers (`bin/bin/api.rs:452-455`), plus the public webhooks and the OAuth callback. It applies identity buckets when a principal exists and IP buckets always. Outside `Local` a Redis store is mandatory (`bin/bin/api.rs:131-140`), so the layer cannot silently fall away in prod.
- **Write group.** `item_writes` allows 30/min per subject (`bin/bin/api.rs:36-37`, `456-468`). Its predicate `is_item_write` (`bin/bin/api.rs:599-604`) matches every POST/PUT/PATCH/DELETE in the router, not just `/items`. In practice it is a general authenticated-write limit, and the name misleads.
- **Platform API.** It is merged after the global layer (`bin/bin/api.rs:534`). It has its own per-product bucket, which runs only after a successful token verify (`api/routes/platform.rs:86-120`), so invalid-token attempts against `/platform/v1/*` are not rate limited at all.
- **MCP.** It has its own store (`bin/bin/api.rs:495-505`), also outside the global layer.
- **Dedicated limiters:**
  - invitation sender: `pg/invitations.rs:127-144`, used for org create/resend at `:405` and for family invite/resend at `pg/families.rs:194`, `:459`
  - export: `svc/export.rs:61-66`
  - promo redeem: `svc/promo.rs:84-92`
  - Kompass has a credit quota (`svc/conversations.rs:258-284`). That is a cost cap, not a request rate limit.

Abuse-prone routes with only the global identity/IP buckets (and the write group where the method is a write):

| Route | Exposure |
|---|---|
| POST `/v1/kompass/conversations`, `/conversations/{id}/messages` | LLM; the only extra protection is the credit quota |
| POST `/v1/kompass/connections/{productId}` and the public GET `/v1/kompass/connections/callback` | outbound OAuth |
| GET `/v1/organizations/{id}/invoices`, `/invoices/{invoiceId}/pdf` | outbound Paddle; GET, so outside the write group |
| POST `/v1/checkout/sessions`, `/v1/billing/portal`, `/v1/organizations/{id}/checkout` | outbound Paddle |
| POST `/v1/organizations/{id}/directory/portal-links` | outbound WorkOS |
| POST `/v1/invitations/accept`, `/preview` and the family equivalents | token is a 256-bit hash, so low risk |
| POST `/v1/me/tokens` | capped by an active-token count |
| Public webhooks: Paddle, RevenueCat, WorkOS | IP only; signature-verified |

This product has no sign-up or session endpoints; identity is external OIDC.

### 5. Sync

`baukit-sync` is not used. No crate depends on it and no push/pull routes exist. Not applicable.

### Findings

**Confirmed missing scoping**
- `GET/PUT/DELETE /items/{id}` and `GET /items`. The table has no owner column (`migrations/0001_create_items.sql`), the queries are id-only (`pg/lib.rs:106`, `:163`, `:176`) and the handlers ignore the principal (`api/lib.rs:338-453`). Any hub user can read and change all items. This is a template scaffold that should be removed or scoped.

**Service-check only (latent)**
- Family routes, 10 routes plus checkout with `familyId`. The code loads the family by id and runs the organizer/member check in Rust (`pg/families.rs:129-134`, `:564-577`, `:914-927`, `:950-978`; `pg/billing.rs:393-399`). Only audit and checkout have non-member tests.
- Platform `GET /families/{familyId}/members` loads any family and filters members in Rust (`pg/families.rs:1072-1100`).
- Organization routes, 22 that use `authorize()`. Membership and role are read in SQL, but data queries filter by `organization_id` only, so correctness depends on every repository method calling `authorize` first (`pg/organizations.rs:187-221`). Coverage is strong for the core routes (outsider matrix). Org products, directory groups, invitation create/resend and business checkout/subscription have no outsider tests.
- Existence oracles: `require_world` runs before the membership check (`pg/organizations.rs:205-207`), and family loads happen before the check.

**SQL-scoped (good)**
- All account-owned routes that take an id. Plan-change, actions, conversations, push devices, erasure status, token revoke, credits entries, connections, weekly plans, onboarding shares.
- Org child rows: invitations, directory groups, org tokens.
- Business billing, with SQL membership and role plus subscription `owner_id`.
- All platform routes except family members.
- Org API tokens are pinned to their org in middleware (`api/token_access.rs:41-58`).
- Rate limiting:
  - Gaps: invalid-token attempts on `/platform/v1` are never rate limited, and the outbound Paddle invoice GETs have only the global bucket.
  - The global layer is enforced in non-local environments.
  - The `item_writes` group is in fact an all-writes limit.

---

# hebkit: tenancy and rate-limit audit (reduced depth)

Backend: /home/patrick/projects/hebkit/backend. Steps 1, 3 and 5 in full; steps 2 and 4 are spot checks. All paths below are relative to `backend/`.

## 1. Ownership model

hebkit scopes data per user. The owner key is `users.id` (Uuid), written as `user_id` or `owner_id` on owned tables. Trainer data adds a second axis: `trainer_relationships(trainer_user_id, client_user_id)`, plus `trainer_organizations` for org-level programs and handovers.

- `baukit_auth::establish_principal` sets the `Principal` (crates/hebkit-api/src/adapters/http/mod.rs:289-292). Then `auth::load_profile` (mod.rs:280-283) resolves the subject to a profile and inserts `AuthedUser { profile, access_channel }` (crates/hebkit-api/src/adapters/http/auth.rs:20, inserts at :100, :136, :163, :174, :191).
- Handlers take `Extension(user): Extension<AuthedUser>` and pass `user.profile.user.id` to services as a bare `Uuid`. Examples: `get_exercise` handlers.rs:270-278, `delete_plan` handlers.rs:576-586, `get_session` handlers.rs:659-670. The owner is a bare `Uuid` argument with no newtype, so the type system does not enforce scoping (E4-05 applies).
- Suite routes authenticate separately through `suite::authenticate` (crates/hebkit-api/src/adapters/http/suite.rs:37).

## 2. Route inventory (spot check)

openapi.json lists about 330 operations. Roughly 200 of them take a resource id in the path. I did not enumerate every one. I sampled the repositories:

| Area | Query | Classification |
|---|---|---|
| plans get/update/delete | crates/hebkit-postgres/src/adapters/postgres/plans.rs:362, :432, :657, :1027, :1066 (`WHERE id = $1 AND user_id = $2`) | SQL-scoped |
| sessions | workouts.rs:218, :273, :688 (same pattern) | SQL-scoped |
| integrations connections | integrations.rs:325, :437, :544, :625, :693, :731, :875, :916, :945, :1016, :1201 | SQL-scoped |
| metrics | metrics.rs:92 (`WHERE user_id = $1 AND id = $2`) | SQL-scoped |
| exercise get | mod.rs:527-530 (`AND (owner_id IS NULL OR owner_id = $2)`) | SQL-scoped |
| exercise update/delete | mod.rs:450-465 `assert_custom_exercise_owner`: loads by id `FOR UPDATE`, then compares `owner_id` in Rust | service-check only |
| session set_logs patch | workouts.rs:654-670: loads set by id `FOR UPDATE`, compares `session_id` to a session already loaded with owner scope | service-check only (parent is SQL-scoped) |
| plan days replace | plans.rs:789-797: `SELECT plan_id FROM plan_days WHERE id = $1 FOR UPDATE`, `Conflict` when the plan differs | service-check only (parent is SQL-scoped) |
| stats | stats.rs:66 `WHERE id = $1`, not reviewed further | unverified |
| trainer claim | trainer_claim.rs:262, :309, :360, :695, :778 (`WHERE id = $1`) | unverified. Claim tokens are bearer-style by design; worth a targeted look. |

The id-only lookups I sampled (mod.rs:454, workouts.rs:660, plans.rs:790) are all followed by an ownership or parent comparison. I found no confirmed MISSING case. The trainer routes (`/trainer/clients/{clientId}/...`, `/trainer/organization/{organizationId}/...`) go through relationship/org checks. I did not trace them to SQL, but the authorization-matrix tests in section 3 cover them heavily.

## 3. Tests (cross-tenant by behavior)

I searched for tests that create a second user or org and assert hiding or rejection. 32 test functions match by name, and 40 files contain second-user or foreign-id fixtures. Examples:

- tests/plans.rs:490 `training_plans_are_owner_scoped_for_reads_and_mutations`
- tests/workouts.rs:105 `..._owner_isolation`, tests/workouts.rs:940 (stats exclude other users)
- tests/nutrition.rs:570 and :1056 (nutrition REST, templates and recipes owner-isolated)
- tests/shopping.rs:571 `shopping_ownership_cascades_and_sync_revisions`
- tests/integrations.rs:165 and :429 (connection ownership, disconnect)
- tests/sync.rs:53 `sync_push_applies_lww_and_enforces_ownership`. This test uses a second token (:59, :105) against a training plan only.
- Trainer: tests/trainer_authorization_matrix.rs:2050, :2338; tests/trainer_authorization_matrix/delegated.rs:46; tests/trainer_authorization_matrix/phase_four.rs:182, :804; tests/trainer_authorization_matrix/tokens.rs:303; tests/trainer_programs.rs:326, :1395; tests/trainer_comments.rs:344; tests/trainer_reviews.rs:284; tests/trainer_organizations.rs:624, :1608; tests/trainer_organizations_hub.rs:423; tests/trainer_delegated_sync.rs:550, :647; tests/trainer_claims.rs:955, :1735; tests/mcp/trainer.rs:349.

Numbers: about 200 id-bearing routes. Roughly 15 to 20 resource families have at least one cross-tenant test. These include plans, sessions, nutrition (foods, templates, recipes), shopping, integrations, and the trainer families (clients, programs, org, comments, reviews, tokens, claims). I did not count per-route coverage. Families with no cross-user test found: exercises update/delete (the service-check path), api-tokens for non-trainer users (tests/api_tokens.rs has no second user; trainer tokens are covered), Strava export routes, hydration by id (tests/hydration.rs has 2 weak matches, unconfirmed), and sync push for child entities (only plans are tested).

## 4. Rate limiting (spot check)

All layers are in crates/hebkit-api/src/adapters/http/mod.rs.

- The main authenticated router uses `baukit_ratelimit::layers` with the global identity and IP budget (:284-288). This router covers the core API, chat, reports, the trainer router, listing, plan_routes and catalog.
- The food catalog gets an extra `authenticated_route_group` at 10 requests per 60 s per subject (:219-232).
- Realtime upgrade is limited by IP only, with identity disabled (:258-265).
- Directory (public) is limited at :313-317 and webhooks at :320-324.
- Operator routes have no limiter on purpose (comment at :294, :295-311).
- Suite routes (`/api/v1/suite/*`) are nested at :326-342 with `suite::authenticate` only and no `baukit_ratelimit::layers`. A limiter goes into the suite service (crates/hebkit-bin/src/suite.rs:38, :56). I did not verify which suite endpoints it gates. Suite routes include `links/exchange`, `inbound/{linkId}` and `links/{id}/test`, which make outbound calls.

Abuse-prone routes that only have the shared global budget, with no tighter per-route quota: `POST /nutrition/recipes/import` (fetches a URL), `POST /nutrition/recommendations`, the `oauth/start` routes for every provider, `POST /trainer/invitations`, `POST /trainer-claims/*`, `POST /trainer-invitations/*`, `POST /reports`, Strava export/replace, photo and certificate uploads, `GET /food-catalog/search` (covered by the catalog group), and `POST /sync/push` (body cap plus row caps, see tests/sync.rs:1663). OAuth callbacks and webhooks are covered by the webhook/global layers only as far as they sit in those routers. I did not trace each one.

## 5. Sync push owner isolation

Every user-owned root upsert in crates/hebkit-postgres/src/adapters/postgres/sync.rs uses `ON CONFLICT (id) DO UPDATE ... WHERE <table>.user_id = EXCLUDED.user_id` (or `= $2`). A pushed id that belongs to another user then updates zero rows instead of overwriting. Instances:

- exercises :1842 (`owner_id`), training_plans :1896, workout_sessions :2183, health_metrics :2361
- sleep_sessions :2421, hydration_entries :2476, user_foods :2555, user_food_portions :2626
- meal_copy_batches :2673 block, nutrition_goals :2877 block, questionnaire_responses :2943, meal_templates :2983, recipes :3065
- integration_connections :3147, external_records :3225, domain_external_links :3302
- shopping_lists :3372, shopping_list_items :3442, shopping_list_templates :3485, shopping_list_template_items :3540

Child tables without a `user_id` column are guarded differently:

- plan_days :1943 uses an `ON CONFLICT ... WHERE EXISTS (...)` parent check (:1951). The upserts at :2042 and :2267 use the same EXISTS form (:2057, :2277). These are SQL-scoped.
- meal_template_entries (:3042) and recipe_ingredients (:3107) have no conflict guard. They rely on Rust pre-checks. `reference_exists` covers the new parent (:3025, :3092). A `foreign` lookup by entry id compares the existing row's parent owner (:3034-3037, :3099-3102). The lookups take no `FOR UPDATE`. A parent cannot change owner (parent upserts never change `user_id`), so I see no practical race. Still, this is service-check only. meal_entries (around :2750-2762) follows the same `foreign` pre-check pattern.
- The plan parent lookup for plan_days/plan_entries tombstones runs by id only (:4704, :4719). It is used to group changes; the per-row writes above still enforce ownership.

Delegated trainer sync (crates/hebkit-postgres/src/adapters/postgres/sync/delegated.rs) resolves the relationship with `trainer_user_id=$1 AND client_user_id=$2` (:50). It checks write grants and kind (:81-140) and checks that sessions belong to the relationship (:145-175) before applying changes as the client owner.

Test gap: tests/sync.rs:53 proves the cross-user rejection for training plans only. I found no push test that targets a foreign meal_template_entry or recipe_ingredient id.

## Findings

### Confirmed missing scoping
- None found in the sampled routes or in the sync push path.

### Service-check only (latent)
- Exercise update/delete: crates/hebkit-postgres/src/adapters/postgres/mod.rs:450-465 loads by id, then compares owner in Rust.
- Sync push meal_template_entries and recipe_ingredients: sync.rs:3034-3037 and :3099-3102. The `foreign` pre-check has no lock, and the upsert has no ownership guard (:3042, :3107). meal_entries around :2750 uses the same pattern. None of these has a cross-user test.
- Set logs (workouts.rs:654-670) and plan days replace (plans.rs:789-797) compare against a parent that was loaded with owner scope. Low risk.
- Unverified id-only queries: trainer_claim.rs:262/:309/:360/:695/:778, stats.rs:66.
- Rate limiting: suite routes have no HTTP rate-limit layer (mod.rs:326-342). Operator routes are unlimited on purpose (:294). Recipe import, recommendations, OAuth start and invitations have only the global budget.

### SQL-scoped (good)
- Plans, sessions, integrations, metrics and exercise reads put the owner in WHERE (plans.rs:362 etc., workouts.rs:218 etc., integrations.rs:325 etc., metrics.rs:92, mod.rs:527-530).
- 20 sync root upserts use `ON CONFLICT ... WHERE <table>.user_id = EXCLUDED.user_id`, plus EXISTS-guarded plan children (sync.rs:1842-3540).
- Cross-tenant tests are broad for trainer, org, nutrition, plans, shopping and integrations. Gaps: exercises mutations, non-trainer api-tokens, Strava export, and sync child entities.

---

# leitbild (reduced depth: steps 1, 3, 5 full; 2 and 4 spot checks)

Backend root: `/home/patrick/projects/leitbild/backend`. All paths below are relative to it.

## 1. Ownership model

- Scope is a single user. Tenant-owned tables carry `user_id` (journal_entries, program_runs, reflections, reminder_settings, user_identities) or `owner_user_id` (programs, where NULL means the shared catalog). There is no org or workspace layer.
- Principal type is `baukit_auth::Principal`, put into request extensions by `establish_principal` (layer attached at `crates/leitbild-api/src/lib.rs:106` and `:138`). Handlers take `principal: Principal` as an extractor (for example `crates/leitbild-api/src/api_token.rs:238`).
- Principal to user id goes through `resolve_user_id` at `crates/leitbild-api/src/lib.rs:620`. An API token yields `token.owner_id` (`:636`). An OIDC session goes through `state.users.resolve_subject(...)` (`:624-628`).
- API token grants are checked per path by `authorize_api_token` (`crates/leitbild-api/src/lib.rs:267`, mapping at `:282`).

## 2. Route spot checks (id-taking routes)

Router: `crates/leitbild-api/src/lib.rs:144-237`. The id-taking routes are /me/erasures/{operationId}, DELETE /me/api-tokens/{id}, /journal-entries/{id} (GET/PUT/PATCH/DELETE), POST /journal-entries/{id}/reflection, /reflection-jobs/{id}, /reflections/{id}, /programs/{id}, /me/templates/{id} (PUT/DELETE), /program-runs/{id} (GET/DELETE), /program-runs/{id}/resume, PUT /program-runs/{runId}/sections/{sectionId}/response, POST /program-runs/{runId}/stages/{stageId}/complete and POST /program-runs/{id}/complete. That makes 13 paths and 18 method routes.

| Route | Query (path:line) | Class |
|---|---|---|
| GET /journal-entries/{id} | `crates/leitbild-postgres/src/journal.rs:199` `entry.user_id = $1 AND entry.id = $2` | SQL-scoped |
| PUT/PATCH /journal-entries/{id} | `journal.rs:271`, `:288`, `:332`, `:348` `id = $1 AND user_id = $2` | SQL-scoped |
| DELETE /journal-entries/{id} | `journal.rs:390`, `:424` | SQL-scoped |
| POST /journal-entries/{id}/reflection | `crates/leitbild-postgres/src/ai.rs:114` | SQL-scoped |
| GET /reflections/{id} | `ai.rs:376` `r.user_id = $1 AND r.id = $2` | SQL-scoped |
| GET /reflection-jobs/{id} | `ai.rs:397` `j.id = $1 AND j.payload->>'user_id' = $2` | SQL-scoped (scope comes from the JSON payload, not a column) |
| GET /programs/{id} | `crates/leitbild-postgres/src/program.rs:248` `owner_user_id IS NULL OR owner_user_id = $2` | SQL-scoped |
| PUT/DELETE /me/templates/{id} | `program.rs:687` `id = $1 AND owner_user_id = $2 AND is_active FOR UPDATE` | SQL-scoped |
| PUT .../sections/{sectionId}/response | gate `crates/leitbild-postgres/src/program_run.rs:231` (`r.user_id = $2`, `FOR UPDATE OF r`), then the upsert at `:255-264` and re-read at `:289` filter by `run_id` only | SQL-scoped through the locked gate in the same transaction |
| program-runs complete, stage complete, resume, delete | `program_run.rs:318`, `:337`, `:387`, `:421`, `:497`, `:520`, `:877` all `run.id = $1 AND run.user_id = $2` | SQL-scoped |
| DELETE /me/api-tokens/{id} | handler passes `user_id` to `state.api_tokens.revoke(user_id, token_id)` (`crates/leitbild-api/src/api_token.rs:242-245`). The SQL lives in the baukit-auth token store and I did not read it here | Scoped by argument, SQL not verified |
| GET /me/erasures/{operationId} | `crates/leitbild-api/src/lib.rs:591`, not traced | Not checked |

Id-only queries I sampled that are not reachable with an attacker-chosen id:
- `crates/leitbild-postgres/src/journal.rs:774` and `:781` (`WHERE id = $1`) use `entry.id`, which was just inserted in the same transaction for the authenticated user.
- `crates/leitbild-postgres/src/lib.rs:93`, `:130` and `:143` touch the template `items` table by id only. `ItemService` is still in `ApiState` (`crates/leitbild-api/src/lib.rs:53`), but no `/items` route is mounted in `routes()`. It is dead scaffold code that would turn into a MISSING finding if someone mounted it again.
- `crates/leitbild-postgres/src/program.rs:416-428` upserts `programs` with `ON CONFLICT (id) DO UPDATE SET owner_user_id = EXCLUDED.owner_user_id`. This is the catalog import path (it deactivates rows by `slug` at `:410`). User template writes go through `update_owned` (`:672`, check at `:687`). If a user-controlled id ever reached this upsert, it could reassign ownership of a row.

I found no MISSING case in the sample.

## 3. Cross-tenant tests

The fixture `memory_app()` (`tests/api_integration.rs:2066-2068`) returns an `owner_token` and a `foreign_token` for two distinct subjects. Tests that use the foreign token against an owner's resource and assert 404:

- GET /journal-entries/{id}: `tests/api_integration.rs:1351` (memory) and `:1890` (Postgres)
- GET /program-runs/{id} and /resume: `tests/api_integration.rs:1488`
- GET /reflection-jobs/{id}: `tests/api_integration.rs:1117`
- Starting a run from another user's template: `tests/api_integration.rs:1746` (test at `:1713`)
- MCP account isolation: `tests/mcp_integration.rs:137`
- Push device ownership: `tests/api_integration.rs:426-468` (no id in the path)

Numbers: 18 method routes take an id. 4 of them have a cross-tenant test (GET journal entry, GET run, GET resume, GET reflection job). Untested cross-tenant: PUT/PATCH/DELETE journal entry, POST entry reflection, GET /reflections/{id}, GET /programs/{id} for a foreign owned template, PUT/DELETE /me/templates/{id}, DELETE run, section response, stage complete, run complete, api-token revoke and erasure status.

## 4. Rate limiting (spot check)

- `baukit_ratelimit::layers(routes(state), ...)` wraps the main `/v1` router (`crates/leitbild-api/src/lib.rs:138`). Every route in `routes()` is under it, including AI reflection requests and api-token creation.
- The layer is conditional. `crates/leitbild-bin/src/bin/api.rs:215-218` uses `rate_limited_routes` only when `request_rate_limiter` is `Some`. Otherwise it falls back to `authenticated_routes(state)`, which has no request limiter at all.
- Suite, MCP and platform routers are merged after the layer (`api.rs:219`), so `layers()` does not cover them. MCP gets its own store (`api.rs:177-198`). I did not check whether the suite routes get one (a store is built at `api.rs:89-91`).
- A second limiter covers writes: `WriteRateLimiter` (Redis or in-memory, `api.rs:112-118`). The worker also builds a limiter (`crates/leitbild-bin/src/bin/worker.rs:71-75`).
- Test: `tests/rate_limit_integration.rs:29` checks that the Redis limit applied to one identity leaves another identity alone.

## 5. Sync

leitbild does not use baukit-sync. No crate's `Cargo.toml` lists `baukit-sync`, and no code mentions `baukit_sync`. Push/pull isolation does not apply.

## Findings

Confirmed missing scoping
- None in the sampled routes.

Service-check only (latent)
- None found. One related risk: the `items` repository is id-only (`crates/leitbild-postgres/src/lib.rs:93`, `:130`, `:143`). It is dead code today because no route is mounted.
- The catalog upsert `crates/leitbild-postgres/src/program.rs:416-428` can rewrite `owner_user_id` on an id conflict. Only import code reaches it, so this is safe as long as user ids never get there.
- `/reflection-jobs/{id}` scopes through `payload->>'user_id'` (`crates/leitbild-postgres/src/ai.rs:397`) instead of a typed column.
- The request rate limiter is optional. When none is configured, `/v1` runs unlimited (`crates/leitbild-bin/src/bin/api.rs:215-218`), and suite/platform routes sit outside `layers()`.

SQL-scoped (good)
- Journal, reflection, program, template and program-run queries put `user_id`/`owner_user_id` in the WHERE clause or behind a `FOR UPDATE` gate on the owned row. The tests cover only 4 of the 18 id routes against a foreign user. Writes and deletes are not covered.

---

# redemut: tenancy and rate-limit audit (reduced depth)

Backend: `/home/patrick/projects/redemut/backend`. Paths below are relative to it. Steps 1, 3 and 5 were done fully. Steps 2 and 4 are spot checks.

## 1. Ownership model

redemut is single-user tenancy. There is no org or workspace scoping of product data. Every owned row carries `user_id UUID` referencing `users(id)` (`migrations/20260730000000_initial.sql:1-45`, `migrations/20260801000000_user_content.sql`). `users` is keyed by `(issuer, subject)` (`initial.sql:6`).

- The principal is `redemut_ports::VerifiedIdentity` (`crates/redemut-ports/src/lib.rs:656`). It holds `identity: Identity {issuer, subject}` (`crates/redemut-domain/src/lib.rs:22`). It also has `organization: Option<String>`, which product data never uses.
- `redemut_auth::authenticate` (`crates/redemut-auth/src/lib.rs:558`) inserts it as a request extension. It is attached as a `route_layer` on the protected router (`crates/redemut-bin/src/application.rs:447`). Handlers take `Extension<VerifiedIdentity>`, for example `crates/redemut-api/src/lib.rs:1982`.
- Services resolve the internal `user.id` with `users.find_or_create(identity...)`, for example `crates/redemut-services/src/user_content.rs:369-372`. They pass it to the repo as the first argument. No client-supplied user id reaches a query.

## 2. Route inventory (spot check)

The protected id routes come from `protected_base_router` and `ugc_write_router` (`crates/redemut-api/src/lib.rs:3312-3404`).

| Route | Repo SQL | Class |
|---|---|---|
| GET `/v1/me/content/dialogs/{id}` | `crates/redemut-postgres/src/lib.rs:1625` `WHERE user_id = $1 AND id = $2` | SQL-scoped |
| PUT `/v1/me/content/dialogs/{id}` | `lib.rs:1800`, if-match `1843`/`1853` | SQL-scoped |
| DELETE `/v1/me/content/dialogs/{id}` | `lib.rs:1827` → `1843`/`1853` | SQL-scoped |
| GET `/v1/me/content/word-packs/{id}` | `lib.rs:1647` | SQL-scoped |
| PUT `/v1/me/content/word-packs/{id}` | `lib.rs:1980`, `2012` | SQL-scoped |
| DELETE `/v1/me/content/word-packs/{id}` | `lib.rs:2039` → `2055`/`2065` | SQL-scoped |
| DELETE `/v1/account/devices/{id}` | `lib.rs:684` `WHERE user_id = $1 AND id = $2` | SQL-scoped |
| POST `/v1/sync` (body device/event ids) | see section 5 | mostly SQL-scoped, one gap |
| `/v1/me/learning-plan`, `/v1/me/calendar/feed` | singleton per user, no id | n/a |
| `/v1/content/bundles/{id}`, `/v1/content/audio/{assetId}` | catalog content, not tenant-owned | n/a |

I did not trace `POST /v1/dialog/coach/turn` or the suite/platform routers (`suite_router_with_group_layers`, `account_plan_router`). Every sampled query puts the owner in the WHERE clause, and I found no "load by id, then compare in the service" pattern among them.

## 3. Tests

I searched for tests that create two principals and assert 404/403.

- `tests/it_user_content.rs:773` (`ownership_is_hidden`). It uses two tokens. User B's list omits A's dialog, and B's PUT and DELETE on `/v1/me/content/dialogs/{id}` return 404.
- `crates/redemut-api/src/lib.rs:7708` (`forget_device_returns_404_for_another_users_device`). It is a unit test with an in-memory repo.
- `tests/it_mcp.rs:330-386`. `another-account` gets `not_found` from the MCP `get_{kind}` tool and the related tools, for both dialogs and word packs.
- `crates/redemut-postgres/tests/postgres_repositories.rs:344` uses `user_a`/`user_b`, but for usage accounting, not access.

Of the 7 HTTP routes with a path id, 3 have a cross-tenant HTTP test (dialog PUT and DELETE, device DELETE). Dialog GET and all three word-pack routes have none over HTTP, and the MCP tests cover only reads through the tool layer. No test pushes a sync with another user's device id.

## 4. Rate limiting (spot check)

- The baukit-ratelimit global layer wraps the whole protected router (`crates/redemut-bin/src/application.rs:445`). Per-group layers come from `apply_group_rate_limits` (`application.rs:325-331`). The groups are UgcWrites, Sync, Settings, AccountDestructive (this includes the erasure router, `application.rs:340-343`), CalendarIssue, Transcription (`application.rs:376`, STT/upload) and Coach (`application.rs:407`, LLM).
- The public calendar feed is wrapped separately (`application.rs:420-424`).
- The MCP server has its own write limiter (`crates/redemut-mcp/src/rate_limit.rs`, wired at `application.rs:425-437`). It also receives the shared store (`application.rs:456`).
- `public_router` (health, capabilities, manifest, `content/bundles/{id}`) and `platform.receiver` (webhook) are merged without a limiter (`application.rs:462-467`). Neither is a sign-up, AI or upload path. Sign-up does not apply here, because `users` rows are created lazily on the first authenticated call.
- I did not check whether `suite_router_with_group_layers` puts a group on every suite route (invite/link endpoints).

## 5. Sync owner isolation

`SyncService::sync_inner` lives at `crates/redemut-services/src/lib.rs:1235`.

- The device upsert checks ownership. `upsert_device` (`crates/redemut-postgres/src/lib.rs:613-675`) selects the existing row `FOR UPDATE` and returns `Conflict("device ... belongs to another user")` when `user_id` differs (`lib.rs:629-634`). The `ON CONFLICT ... WHERE devices.user_id = EXCLUDED.user_id` at `lib.rs:665` adds a second guard.
- Item states and settings always use the server's `user.id` (`crates/redemut-services/src/lib.rs:1323-1330`). Their primary keys are `(user_id, item_ref, kind)` and `user_id` (`initial.sql:37,41`), so a push cannot overwrite another user's row.
- Events use the server's `user.id` (`services/src/lib.rs:1298`). The primary key is `(user_id, device_id, seq)` and inserts are `ON CONFLICT ... DO NOTHING`, so another user's events cannot be overwritten.
- Gap: each event's `device_id` comes from the client (`services/src/lib.rs:1299`). `validate_push` (`services/src/lib.rs:1400-1450`) never checks that it equals `request.device.id` or belongs to the caller. The FK is global: `learning_events.device_id REFERENCES devices(id) ON DELETE NO ACTION` (`migrations/20260901000000_storage_usage.sql:15-17`). User B can therefore store events that point at user A's device UUID. B needs to know that UUID, but device ids are client-generated and not secret by design. A's rows are not exposed or overwritten. However, deleting A's device row (account erasure cascade via `users`) would then hit B's NO ACTION reference and fail. I inferred this from the schema and did not run it.

## Findings

Confirmed missing scoping
- None among the sampled routes.
- Partial: sync event `device_id` is not checked against the caller (`crates/redemut-services/src/lib.rs:1299`, `1400-1450`). The global FK at `migrations/20260901000000_storage_usage.sql:15-17` allows cross-tenant references and may block the other user's erasure. Fix with `event.device_id == request.device.id`, or with a composite FK `(user_id, device_id)` to `devices(user_id, id)`.

Service-check only (latent)
- `upsert_device` loads by id and compares `user_id` in code (`crates/redemut-postgres/src/lib.rs:619-634`). The `ON CONFLICT WHERE` at `lib.rs:665` backs it up, so it is low risk.
- No other cases found in the sample.

SQL-scoped (good)
- All user dialog and word-pack GET/PUT/DELETE queries (`crates/redemut-postgres/src/lib.rs:1625,1647,1800,1843,1853,1980,2012,2055,2065`).
- Device forget (`lib.rs:684`).
- Item state, settings and event upserts keyed by the server-resolved `user_id`.
- Test gap: dialog GET and all word-pack routes lack a cross-tenant HTTP test (3 of 7 id routes are covered, `tests/it_user_content.rs:773`, `crates/redemut-api/src/lib.rs:7708`).

---

# schlauzug (backend) tenancy and rate-limit audit

Reduced depth. Steps 1, 3 and 5 in full; steps 2 and 4 as spot checks. All paths are relative to `/home/patrick/projects/schlauzug/backend`.

## 1. Ownership model

Data belongs to one user, `Uuid` `user_id` / `owner_user_id` (runs, question_sets) / `host_user_id` (rooms). There is no org or workspace tenant.

- Principal: `baukit_auth::Principal`, which `establish_oidc` (`crates/schlauzug-api/src/lib.rs:1318`, attached at `lib.rs:905`) puts in the request extensions. Handlers take it either as `Principal` or as `Option<Extension<Principal>>`.
- Subject to user id: `required_owner` (`lib.rs:2410`), and inside `authorize_http` it calls `users.resolve_subject` (`lib.rs:2375-2382`).
- There are two more ways in, both bearer capability tokens. A run token `srt_` is checked against `runs.token_hash` (`crates/schlauzug-services/src/run_service.rs:1107-1109`). A room seat token `sst_` is looked up by `token_hash` (`crates/schlauzug-postgres/src/lib.rs:1130`).

## 2. Route spot check (id-taking routes)

The router is at `crates/schlauzug-api/src/lib.rs:837-884`. The `/v1` routes are listed below. I did not walk every entry in `openapi.json` (47 `/v1` lines).

| Route | Path to the check | Classification |
|---|---|---|
| `GET/PUT/DELETE /v1/me/question-sets/{id}`, `/publish`, `/duplicate` | handlers `lib.rs:1475-1539` pass `owner` to the repo. SQL `WHERE id = $1 AND owner_user_id = $2` at `postgres/src/lib.rs:1437, 1475, 1494, 1526, 1582, 1627, 1665` | SQL-scoped |
| `GET /v1/question-sets/{id}`, `/questions` | `question_set_detail(&id, Some(owner))` `lib.rs:1285` -> `resolve_playable_set` `run_service.rs:630-636` (public, or own drafts) | Visibility rule in the service. I did not trace it to SQL |
| `GET /v1/runs/{id}`, `POST /v1/runs/{id}/commands`, `GET /v1/runs/{id}/live` | `authorize_http` `lib.rs:2368` -> `RunService::authorize` `run_service.rs:1095-1115`. `repository.load(run_id)` uses SQL `FROM runs WHERE id = $1` (`postgres/src/lib.rs:606`), then compares `owner_user_id` / token hash at `run_service.rs:1106-1110` | Service-check only (latent) |
| `GET /v1/rooms/{id}`, `/live` | `authorize_room_view` `lib.rs:2349` -> `room_service.rs:1030-1040`. It accepts a seat token, or `load(room_id)` (`FROM rooms WHERE id = $1`, `postgres/src/lib.rs:1763`) followed by a `host_user_id` compare | Service-check only (latent) |
| `POST /v1/rooms/{id}/commands` | `authorize_seat(id, token)` `lib.rs:2280-2284`. Seat lookup `WHERE token_hash = $1 AND removed = false` (`postgres/src/lib.rs:1130`) | Capability token. I did not confirm that the seat's room_id is compared with the path id |
| `GET /v1/me/erasures/{operationId}` | `store().status(principal.subject(), operation)` `lib.rs:3347-3365` | Subject-scoped (baukit erasure store) |
| `/v1/me/*` reads and workshop writes | user_id from the principal. SQL `WHERE user_id = $1` throughout `postgres/src/lib.rs` (e.g. 2988, 3047, 3251) | SQL-scoped, no client-supplied id |

I found no MISSING routes in the sample.

## 3. Tests (cross-owner behaviour)

- `tests/api_question_sets.rs:928` `own_routes_require_oidc_and_hide_other_owners_sets` gives a second owner 404 (line ~952) and 403 (line ~982). It covers the `/v1/me/question-sets/{id}` family.
- Runs: `tests/api_runs.rs:476` (a run token cannot reach account routes) and `:507` (token-based view/command). I found no test where user B uses user A's run id with B's own OIDC token or the wrong `srt_`. I also found no `authorize(` call in the services' `*tests*.rs` files.
- Rooms: `tests/api_rooms.rs:1314` (a removed seat's token gets 401 at line 1337) and `tests/api_rooms/crater_duel.rs:1377` (team views stay private). I found no test of a non-host OIDC user viewing another host's room by id.
- Numbers: about 13 id-taking routes. 5 of them (the own question-set family) have a cross-owner test, and the room-seat routes have partial token coverage. The run routes and the OIDC host-view path have none.

## 4. Rate limiting (spot check)

- baukit-ratelimit request limits per identity and per IP: `apply_request_limits` -> `baukit_ratelimit::layers` (`lib.rs:167-171`), applied to the whole router at `lib.rs:903-904`, inside OIDC and CORS.
- Product limiter: `product_rate_limit` middleware (`lib.rs:1057-1120`, attached at `lib.rs:893`). It has buckets for room join per IP (fail-closed by default, `crates/schlauzug-bin/src/lib.rs:374-381`), gameplay commands per IP plus per account or token, question-set creation, per-subject writes, account reads, and `srt_`/`sst_` token routes. WebSocket upgrades get their own `socket_upgrades_per_ip` cap (`schlauzug-bin/src/lib.rs:530`).
- Gaps: no product bucket covers anonymous non-join writes. They fall through to `LimitOutcome::Allowed` at `lib.rs:1112-1113` and only the generic baukit per-IP layer applies. There is no auth or sign-up endpoint here (OIDC is external), and there are no upload, LLM or outbound-integration routes. `DELETE /v1/me` (erasure) only gets the generic write budget.

## 5. Sync

baukit-sync is not used. No crate depends on it and no push/pull routes exist, so this step does not apply.

## Findings

**Confirmed missing scoping**
- None found in the sampled routes.

**Service-check only (latent)**
- Runs `GET /v1/runs/{id}`, `/commands`, `/live`: `FROM runs WHERE id = $1` (`crates/schlauzug-postgres/src/lib.rs:606`), owner/token compare at `crates/schlauzug-services/src/run_service.rs:1106-1110`. No cross-user test.
- Rooms `GET /v1/rooms/{id}`, `/live` via OIDC host path: `FROM rooms WHERE id = $1` (`crates/schlauzug-postgres/src/lib.rs:1763`), host compare at `crates/schlauzug-services/src/room_service.rs:1040`. No cross-host test.
- Room commands depend on a seat-token lookup (`crates/schlauzug-postgres/src/lib.rs:1130`). I did not verify that the seat's room matches the path id.
- Public question-set detail visibility lives in `resolve_playable_set` (`crates/schlauzug-services/src/run_service.rs:636`). I did not trace it to SQL.

**SQL-scoped (good)**
- Owned question-set routes (`owner_user_id = $2` in every query, covered by `tests/api_question_sets.rs:928`).
- All `/v1/me/*` profile, progression, workshop, wallet and history queries (`WHERE user_id = $1`, user id taken from the principal).
- Erasure status is scoped by principal subject (`crates/schlauzug-api/src/lib.rs:3359-3362`).

---

# tiefgang: tenancy and rate-limit audit (reduced depth)

Backend: `/home/patrick/projects/tiefgang/backend`. Steps 1, 3 and 5 are complete. Steps 2 and 4 are spot checks only. Paths below are relative to `backend/crates/` unless they start with `tests/`.

## 1. Ownership model

Tenancy is per user. There are no orgs or workspaces. Every owned table has `user_id UUID` (domain structs such as `tiefgang-domain/src/project.rs:11` and `tiefgang-domain/src/session.rs:72`), and repositories take an `owner_id: Uuid`.

The principal comes from middleware, not an axum extractor. `authenticate_request` is attached at `tiefgang-api/src/lib.rs:345-348`. It inserts `AuthContext` (`tiefgang-api/src/auth.rs:26-32`), which carries a `baukit_auth::Principal`, an optional `user_id` (set for API tokens), scopes and the credential kind. `ChainedAuthVerifier::authenticate` (`auth.rs:54`) tries OIDC first and falls back to an API token, which it resolves to its owner by hash (`auth.rs:74-79`). Handlers take `Extension<AuthContext>` and map it to the local user with `profile_for(&state, &context)`, for example `tiefgang-api/src/sync_handlers.rs:54-59`. That resolved `profile.user.id` is the `owner_id` handed to every service and repository call.

## 2. Route inventory (spot check)

The router is at `tiefgang-api/src/lib.rs:122-191` for unlimited-class protected routes and `lib.rs:208-338` for class-limited ones. Routes that take an id are `projects/{id}`, `sessions/{id}` with its pause/resume/complete/abort actions, `rules/{id}` (PUT/DELETE), `guard/profiles/{id}` with activate, `api-tokens/{id}`, `webhooks/{id}` with test/deliveries/rotate-secret/reenable, `integrations/{provider}/connections/{connectionId}`, `me/erasures/{operationId}`, the suite `links/{id}` routes from baukit-suite, and the body-id routes `sync/push` and `PUT rules/{id}`.

| Area | Representative query | Classification |
|---|---|---|
| projects get/patch | `tiefgang-postgres/src/projects.rs:53,154,169` (`id = $1 AND user_id = $2`) | SQL-scoped |
| sessions get/actions | `tiefgang-postgres/src/sessions.rs:262,438,459,492,767` | SQL-scoped |
| rules delete/get | `tiefgang-postgres/src/rules.rs:113,126,145,184` | SQL-scoped |
| rules PUT (client-chosen id) | `rules.rs:48` loads by `id` only, then compares owner at `rules.rs:54-56` and returns NotFound | Service-check only (latent, but in the same tx with `FOR UPDATE`) |
| guard profiles put | `tiefgang-postgres/src/guard.rs:88` loads by `id` only, then compares at `guard.rs:94-96` and returns Conflict | Service-check only (latent) |
| guard helper `ensure_owned` | `guard.rs:739` loads by `id`, compares at `guard.rs:745-748` | Service-check only (latent) |
| guard profile get/delete/activate | `guard.rs:200,216,262,276,325,351,365,442,464,758` | SQL-scoped |
| webhooks per id | `tiefgang-postgres/src/webhooks.rs:97,119,148,198,267,285` | SQL-scoped |
| webhooks worker updates | `webhooks.rs:325,341` (`record_success`/`record_failure` by `id` only) | Internal worker path, no route. Acceptable. |
| integration disconnect | `tiefgang-postgres/src/integrations.rs:239` | SQL-scoped |
| integration connection by id | `integrations.rs:333` (`connection`), `integrations.rs:398` (vault `load`) by `id` only | Service-check only. Callers pass ids from an owner-scoped lookup or the public GitHub webhook resolver (`tiefgang-api/src/integration_handlers.rs:238-240`), which authenticates by HMAC signature. |
| api token revoke | not traced (spot check) | n/a |

I found no MISSING case among the sampled queries. I did not trace every handler, the suite `links/{id}` routes or `api-tokens/{id}`.

## 3. Tests (behavioral)

I counted about 18 authenticated route paths that take a resource id (path or body). Three have an HTTP cross-tenant test:

- `PUT /api/v1/rules/{id}`: a second user's PUT on an existing rule id returns 404 (`tests/postgres_integration.rs:2954-2964`).
- `DELETE /api/v1/integrations/{provider}/connections/{connectionId}`: cross-user delete returns 404 (`tests/integrations_integration.rs:174-179`, test `token_connections_are_encrypted_oidc_only_and_owner_scoped` at line 47).
- MCP tools: `remote_router_enforces_oauth_scopes_and_account_ownership` (`tests/mcp_integration.rs:19`) covers MCP, not the REST routes.

Adjacent coverage: `webhook_recovery_replays_encrypted_secret_and_checks_owner` (`tests/worker_integration.rs:210`), `suite_erasure_removes_jobs_and_owner_rows_and_preserves_another_owner` (`tests/suite_integration.rs:1296`), and `owner_foreign_keys_and_owner_lock_block_orphaned_inbound_writes` (`tests/suite_integration.rs:1474`).

No HTTP cross-user test covers projects, sessions and their actions, guard profiles, webhooks per id, api-token revoke, erasure status, or sync push with a foreign entity id. In numbers, roughly 2 of 18 REST id routes are tested across tenants.

## 4. Rate limiting (spot check)

- **All authenticated routes.** `baukit_ratelimit::layers` wraps the whole protected router (`tiefgang-api/src/lib.rs:340-344`). The auth middleware sits outside it (`lib.rs:345-348`), so limits key on the authenticated user.
- **Per-class limits.** `class_limited` (`lib.rs:423-431`) adds stricter buckets for SyncPush (`lib.rs:211-215`), CreateCore (projects/sessions create, `lib.rs:221-226`), PutConfig, GuardIngest, Tokens (`lib.rs:259-263`), Webhooks (`lib.rs:269-288`), Integrations (token connect, Google OAuth start, calendar sync, GitHub connect, `lib.rs:292-306`), Economy (credits spend/grant, panic unlock), EventsIngest (health events), and Erasure (`DELETE /me` via `baukit_ratelimit::authenticated_route_group`, `lib.rs:434-442`).
- **Suite module.** baukit-suite routes get their own `baukit_ratelimit::layers` (`lib.rs:365-369`).
- **Public routes.** The Google OAuth callback and `POST /integrations/github/webhooks/{connection_id}` (`lib.rs:350-359`) use `PublicRouteRateLimits` through `limit_resolved_owner` (`integration_handlers.rs:292-317`). The limiter is charged only after the owner resolves, so requests with unknown connection ids or OAuth states hit the DB with no limit (`integrations.rs:333`, `integrations.rs:93`). This is low severity, an unauthenticated DB-lookup flood.
- **No limit.** `GET /api/v1/openapi.json` is static and harmless. There are no sign-up, invite, upload, search or LLM routes. Sign-up happens implicitly on first OIDC call and goes through the general authenticated limiter.

## 5. Sync push/pull owner isolation

baukit-sync is used. `push` (`tiefgang-api/src/sync_handlers.rs:52-74`) requires an OIDC session and passes `profile.user.id` as owner.

- **Pre-check.** `tiefgang-postgres/src/sync.rs:901-907` looks up the existing row with `WHERE id = $1 AND user_id = $2`. A foreign id therefore looks like a new row.
- **Upsert guard.** Every upsert is `ON CONFLICT (id) DO UPDATE ... WHERE <table>.user_id = $2`: rules `sync.rs:1324-1328`, weekly_reviews `1413`, guard_profiles `1662`, guard_targets `1793`, block_attempts `1925`, block_attempt_buckets `2025`, guard_events `2119`, projects `2217`, focus_sessions `2377`, session_events `2468`. If another user owns the id, the update is skipped. **Owner isolation holds: a push cannot overwrite another user's row.**
- **Error path.** In that foreign-id case the statement returns zero rows, and `.fetch_one` (for example `sync.rs:1340`) raises `RowNotFound`, which maps to a storage error. The probable result is a 5xx and a rolled-back batch, not a per-change rejection. This is a robustness gap and possibly an id-existence oracle, not a data leak.
- **Parent ownership.** Child rows check the parent's owner with `parent_exists(transaction, table, owner_id, id)`: guard_targets to profile at `sync.rs:1782`, block_attempts and buckets to session at `sync.rs:1913` and `sync.rs:2013`, defined at `sync.rs:2142`.
- **No test.** No test pushes a foreign-owned entity id.

## Findings

### Confirmed missing scoping
- None among the sampled queries and the full sync path.

### Service-check only (latent)
- `tiefgang-postgres/src/rules.rs:48` (rules upsert loads by id, owner compared in Rust at `:54`).
- `tiefgang-postgres/src/guard.rs:88` (guard profile upsert, compared at `:94`).
- `tiefgang-postgres/src/guard.rs:739` (`ensure_owned` helper).
- `tiefgang-postgres/src/integrations.rs:333` and `:398` (connection and vault load by id only, which relies on callers passing owner-resolved or HMAC-verified ids).
- Sync push of a foreign-owned id. Isolation holds, but `fetch_one` turns the case into a storage error and probably a 5xx (`sync.rs:1340` and siblings). Map it to a rejected change.
- Test gap: only `PUT rules/{id}` and the integration disconnect have HTTP cross-user tests. Projects, sessions, guard profiles, webhooks, api tokens and sync push have none.
- Public GitHub webhook and OAuth callback lookups with an unknown id or state get no rate limit (`integration_handlers.rs:292-317`).

### SQL-scoped (good)
- projects, sessions and their actions, rules get/delete, guard profiles get/delete/activate, webhooks per id, integration disconnect and status, and all ten sync upserts with their pre-check include `user_id = $2` in the WHERE clause. A global per-user `baukit_ratelimit` layer plus class limits covers all authenticated routes.

---

# runtime-analyzer (finops) tenancy and rate-limit audit

Reduced depth: steps 1, 3, 5 full; steps 2 and 4 spot checks. Backend at `/home/patrick/projects/runtime-analyzer/backend`. Nothing in the repo was modified.

## 1. Ownership model

- The scope key is the tenant, typed as `TenantId(Uuid)` (`crates/finops-domain`). Inside a tenant there is a second layer of `AccessScope` grants (org/product/auditor scopes, `crates/finops-domain/src/access.rs:69`).
- The principal is `FinopsPrincipal` (`crates/finops-api/src/auth/principal.rs:18`, field `tenant_id` at :19). Its extractor is `impl FromRequestParts<ApiState> for FinopsPrincipal` at `principal.rs:346`. It reuses the value the rate-limit middleware already resolved, or calls `FinopsPrincipal::resolve`. API keys are checked against both owner and tenant at `principal.rs:60`. Inactive or wrong-tenant users are rejected at `principal.rs:159`.
- Enforcement happens in the database, which differs from the E4-05 pattern. Repositories run their queries in `TenantScoped::run*` (`crates/finops-postgres/src/tenant_scoped.rs:9-66`). That helper opens a transaction and runs `set_config('app.current_tenant', $1, true)` (:25, :38, :58). The tables carry `ENABLE` + `FORCE ROW LEVEL SECURITY` with `USING/WITH CHECK (tenant_id = current_setting('app.current_tenant', true))`, for example `migrations/0016_rls.sql:29-31` and the loop form at `migrations/0064_evidence_manifests.sql:137`. The test DB role is created `NOSUPERUSER NOBYPASSRLS` (`crates/finops-test/src/shared_timescale.rs:114`). Migrations contain 130 ENABLE and 134 FORCE statements. Composite FKs `(tenant_id, id)` block cross-tenant references, for example `migrations/0063_compliance_catalogs.sql:174,217`.
- If `app.current_tenant` is unset, `current_setting(..., true)` returns NULL, so an unscoped `&self.pool` query on an RLS table sees zero rows and fails closed.

## 2. Route inventory (spot check)

openapi.json has 270 paths. 147 of them carry a path parameter, for 192 operations with an id. I did not enumerate them all. I sampled the id-only SQL (`WHERE id=$1` with no `tenant_id`):

| Query | Context | Classification |
|---|---|---|
| `crates/finops-postgres/src/compliance.rs:485` `compliance_imports WHERE id=$1` | inside `TenantScoped::run(&self.pool, tenant.0, ...)` (:483); table has RLS and a tenant-keyed index (`migrations/0063_compliance_catalogs.sql:320`) | RLS-scoped (no WHERE tenant, DB enforces) |
| `crates/finops-postgres/src/compliance.rs:373` `artifact_uploads WHERE id=$1 AND ...` | inside a tenant-scoped tx; `artifact_uploads` FORCE RLS (`migrations/0074_incident_receipt_uploads.sql:9`) | RLS-scoped |
| `crates/finops-postgres/src/artifact_uploads.rs:128` `UPDATE artifact_uploads ... WHERE id=$1` | tx helper; same RLS table | RLS-scoped |
| `crates/finops-postgres/src/compliance.rs:177,230,256,288` `assessment_profiles` / `framework_versions WHERE id=$1` | public curated catalog, only written when `tenant.is_none()` (:176). Tenant copies live in `tenant_*` tables with RLS (`0063_compliance_catalogs.sql:237,261`) | global data, not tenant-owned |
| `crates/finops-postgres/src/compliance.rs:372` `SELECT count(*) FROM compliance_imports` | the quota count relies on RLS to scope to the tenant | RLS-scoped (implicit) |
| `crates/finops-postgres/src/tickets.rs:290` `integrations WHERE tenant_id=$1 AND id=$2` | explicit tenant plus RLS | SQL-scoped |
| `crates/finops-postgres/src/admin.rs:46,80,102` `api_keys WHERE tenant_id=$1 AND id=$2` | explicit | SQL-scoped |

There are 39 direct `&self.pool` executions in `finops-postgres` without tenant context. The ones I sampled are cross-tenant worker or lookup functions: `tickets.rs:301` (`ticket_poll_candidates`), `agent_tokens.rs:62,69` (`agent_cluster_tenant`). I did not classify the rest. They should be checked against `SECURITY DEFINER` functions, because 62 `SECURITY DEFINER` occurrences in migrations bypass RLS by design.

Caveat: my static parse of the migrations could not confirm FORCE RLS for every table with a `tenant_id` column. Many tables are covered by `FOREACH ... EXECUTE format(...)` loops, for example `0053_runtime_image_inventory.sql:282` covers `workloads`. Checking `pg_class.relforcerowsecurity` against a live DB is the reliable test, and I did not run it.

## 3. Tests

The tests check cross-tenant behaviour, and they do it module by module. 27 test files both create a second tenant and assert 403/404 or empty results. 10 files query `relforcerowsecurity` directly (for example `tests/data_subject_requests.rs`, `tests/privacy_activities.rs`, `tests/ticket_sync.rs`, `tests/siem_export.rs`). Named isolation tests include:

- `tests/data_subject_requests.rs:828` `every_table_forces_rls_and_rejects_cross_tenant_reads_and_writes` (data-subject tables only)
- `tests/incidents.rs:1139`, `tests/report_catalog.rs:1214`, `tests/api_privacy_connections.rs:786`, `tests/access_scopes.rs:64`, `tests/api_sboms.rs:1357`, `tests/ticket_sync.rs:516`, `tests/worker_integration.rs:282`, `tests/terraform_support.rs:720`, `tests/privacy_activities.rs:432`, `tests/evidence_manifests.rs:81`, `tests/identity_erasure.rs:959`.

Numbers: 192 operations take an id. Roughly 15 to 20 feature areas have a cross-tenant test. I did not map coverage per route. I found no single schema-wide test asserting that every table with a `tenant_id` column has FORCE RLS plus a policy, and that gap is the main missing check.

## 4. Rate limiting (spot check)

- baukit-ratelimit is used through a custom middleware, `rate_limit_request` (`crates/finops-api/src/lib.rs:264`). `production_router` attaches it (`lib.rs:93-99`), inside `baukit_auth::establish_principal` (`lib.rs:100-103`). The store is Redis when enabled and falls back to in-memory (`crates/finops-bin/src/bin/api.rs:133-136`, `lib.rs:92`).
- Authenticated traffic gets one bucket per tenant, `tenant:<id>`, sized by tier: 100, 500 or 2000 per minute (`crates/finops-domain/src/tenancy.rs:393-408`). API tokens with an override get an extra per-key bucket (`lib.rs:298-302`).
- Anonymous traffic is limited only on `/auth/*`, by one global key `anonymous:auth` at 120/min (`lib.rs:274-279`). That bucket is shared by every client, so one abuser can lock all sign-ins out. All other anonymous paths skip the limiter (`lib.rs:290`), and so do erasure reconciliation requests (`lib.rs:292`).
- No route has its own budget. Uploads (`/api/v1/artifacts/uploads/{id}/chunks/{index}`), evidence and compliance exports, outbound integrations (Jira/ServiceNow/GRC/SIEM/privacy connections) and search all draw from the same tenant-wide bucket. A noisy user inside a tenant can starve that tenant's other users.
- `router()` (`lib.rs:73`), used by tests, has no limiter. I found no test of the production limiter.

## 5. Sync

baukit-sync is not a dependency of this backend. The `ticket_sync`, `grc_sync` and `privacy_map_sync` test targets cover outbound integration syncs, not client push/pull. N/A.

## Findings

**Confirmed missing scoping**
- None found in the sampled queries.

**Service-check only (latent)**
- None in the classic sense. Many queries use id only (for example `compliance.rs:485`, `artifact_uploads.rs:128`) and depend on RLS alone. That is a sound design, but it means any table that lacks FORCE RLS, or any query routed through `&self.pool` or a `SECURITY DEFINER` function (39 pool call sites, 62 definer occurrences), becomes unscoped without anyone noticing. No schema-wide test catches that.
- Rate limiting: the anonymous `/auth/*` bucket is one global key (`lib.rs:278`), so a single client can exhaust it for everyone. Uploads, exports, outbound integrations and search have no per-route budget.

**SQL-scoped (good summary)**
- The tenant comes from `FinopsPrincipal` (`principal.rs:346`). Repositories run inside `TenantScoped` (`tenant_scoped.rs`). Postgres RLS is FORCEd with a `WITH CHECK` policy, the DB role is NOBYPASSRLS, and composite `(tenant_id, id)` FKs are in place. Several queries also add an explicit `tenant_id` predicate (`admin.rs:46,80,102`, `tickets.rs:290`). 27 test files test cross-tenant behaviour.

---

# architecture-health-platform (reduced depth)

Backend: /home/patrick/projects/architecture-health-platform/backend. Steps 1, 3, 5 full; steps 2 and 4 spot checks.

## 1. Ownership model

- Tenant key is the organization (`organization_id: Uuid`). Nearly every authenticated route is nested under `/organizations/{organizationId}/...` (145 of 148 templated paths in `openapi.json`). Repositories are the second level, keyed `(organization_id, id)`.
- Principal: `baukit_auth::Principal` extractor in handlers (e.g. `crates/architecture-health-platform-api/src/privacy_routes.rs:152`). It maps to an internal user id via `actor()` at `crates/architecture-health-platform-api/src/lib.rs:1326`, which calls `users.resolve_session`.
- Membership gate: `AuthorizationService::require_member` / `require_manager` at `crates/architecture-health-platform-services/src/lib.rs:270` and `:281`, backed by `member_role(organization_id, user_id)`. Ids are plain `Uuid`; no tenant-scoped newtype.
- Typical service pattern: `require_member(user, org)` then repository lookup with org in SQL, e.g. `crates/architecture-health-platform-services/src/maintenance.rs:38-62` and `crates/architecture-health-platform-services/src/external_dependency_reads.rs:68-80`.

## 2. Route spot checks

| Route | Path | Classification |
|---|---|---|
| repository lookup used by most repo routes | `crates/architecture-health-platform-postgres/src/lib.rs:323-330`, `WHERE organization_id=$1 AND id=$2` | SQL-scoped |
| GET maintenance policy | `services/src/maintenance.rs:63-73` -> `current_policy(organization, repository)` | SQL-scoped after membership check |
| GET external dependencies | `services/src/external_dependency_reads.rs:68-90` | SQL-scoped after membership check |
| POST `/privacy/suppressions/{id}/decision` | `api/src/privacy_routes.rs:150-172` builds the decision with `organization_id: org` from the path and `decided_by: user`, then `decide(user, ...)` | org carried into the write; not traced to SQL |

Id-only SQL sample (`WHERE id=$1` with no org): `organizations` (`postgres/src/ci.rs:86`, `incident_providers.rs:109`, `context_import.rs:27`, `automation_jobs.rs:59`), `job_outbox` (`jobs.rs:110`, `ci.rs:53`, `privacy_sync.rs:269`), and `repositories` in `jobs.rs:149`. The `jobs.rs:149` lookup sits inside `JobStore::enqueue` and derives the org from a repository id already in the job payload. It is a worker path, not a request path. The `organizations` lookups key on the tenant itself. None was traced to a user-supplied foreign id, but this was a sample, not a full inventory. 704 SQL predicates bind `organization_id`.

## 3. Tests

36 files in `tests/` reference a second org or non-member (`other_org`, `foreign_org`, `second_org`, `other_organization`, `outsider`, `non_member`, `stranger`). 23 of them also assert `NOT_FOUND` or `FORBIDDEN`. Examples: `tests/maintenance_api.rs`, `tests/remediation_api.rs`, `tests/privacy_connections_api.rs`, `tests/privacy_vendor_api.rs`, `tests/privacy_register_api.rs`, `tests/privacy_gate_api.rs`, `tests/scanner_api.rs`, `tests/runtime_management_api/drift.rs`, `tests/architecture_intent_api.rs`, `tests/flag_connections_api.rs`, `tests/user_profiles_api.rs`, `tests/repository_controls.rs`.
Numbers: 148 templated routes with ids. I did not map tests per route at this depth, so the cross-tenant coverage count is per file (23), not per route.

## 4. Rate limiting (spot check)

- `crates/architecture-health-platform-api/src/rate_limit.rs:14-64`: `baukit_ratelimit::layers` wraps all `authenticated_routes`. Per-subject groups add `management_writes` (30/min), `analysis_triggers` (10/min) and `governance_writes` (30/min), classified in `classify_route` (`rate_limit.rs:72` onward). Webhook routes get a separate IP-only limit of 120/min.
- Extra amount budgets exist for context batches (`api/src/context_batch_routes.rs:17-25`) and inventory ingest (`api/src/inventory_routes.rs:27-50`).
- Abuse-prone routes `/privacy/data-map/exports`, `/privacy/subprocessor-register/uploads`, `/privacy/data-map/push` and `/subprocessor-register/pull` (`api/src/lib.rs:200-204`) get the default layer plus whatever write group they match. None has a dedicated tighter quota. Outbound refreshes are in `analysis_triggers`.

## 5. Sync

No baukit-sync. No `baukit_sync` reference in any crate and no dependency in `crates/architecture-health-platform-bin/Cargo.toml`. `postgres/src/privacy_sync.rs` is a job-outbox sync for the privacy register, not baukit-sync push/pull. Not applicable.

## Findings

Confirmed missing scoping
- None found in the sample.

Service-check only (latent)
- Membership comes from `require_member` in services. Some id-only SQL exists (`job_outbox` and `repositories` in `postgres/src/jobs.rs:110,149`), but only on worker paths. A request path that reached these with a user-supplied id would rely on service discipline alone. Not verified across all 148 routes.

SQL-scoped (good)
- Repository and repo-child lookups bind `organization_id` in SQL (`postgres/src/lib.rs:323-330`, 704 org predicates overall). Org-prefixed routes, a membership gate in services, 23 test files with cross-org 403/404 assertions, and a global plus per-group rate limit.
