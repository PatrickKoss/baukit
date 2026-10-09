# Suite events

Domain and ports crates can depend on `baukit-suite` with `default-features = false`
for peer metadata, catalogs, payload validation, link protocol and serde contracts.
SQLx connection ports and stores require `postgres`. Services require `runtime`,
which enables `postgres`. Enable `http`, `jobs` and `delivery` in the product's
adapter crate to add the router, worker handler and guarded peer client.
See the [crate feature table](../../rust/crates/baukit-suite/README.md#features).

`baukit-suite` links two apps for one user with an OAuth-style code flow and
PKCE S256. Each link has a sealed HMAC secret. Domain writes enqueue signed
events through `baukit-jobs`; receivers verify, validate, deduplicate and apply
them in one transaction. `@baukit/suite-client` supplies the client behavior.
Products own their event types, payload structs, reward rules and UI.

## Protocol

The initiator creates a request with its return URL and client nonce. The
server stores a hashed state and sealed PKCE verifier, then returns the peer's
authorize URL. The authorize page checks the account hint, asks for consent
or approves a matching account, and issues a short-lived code. The callback
checks `from`, relays the code to the recorded return URL and echoes the client
nonce. Only the initiating owner can complete the request. Completion exchanges
the code, verifier, initiator link id and secret with the peer outside the
database transaction, then commits the link and its initial replay.

Requests last ten minutes; codes last one minute. Completion and exchange
retries return the recorded link. Exchange retries must still match the code,
verifier, subjects and initiator link id. A new start supersedes an open request.
Reconnect revokes the old link on both sides. A signed revoke tells the peer to
stop using the link.

Suite subjects have the form `{identity_domain}|{claim}`. The product constructs
them from the authenticated principal with `suite_subject`. Equal domains with
different subjects fail with `suite_link_account_mismatch`. Different domains or an
absent subject store a null shared subject. Never accept a subject from a client
request as authenticated identity.

Events use `baukit_events::EventEnvelope` v1, including `sourceApp`. IDs are
UUIDv5 under `SUITE_EVENT_NAMESPACE`, using `{type}:{natural_key}`. A product's
live and replay builders must use the same natural key, time precision and
payload. PostgreSQL stores timestamps at microsecond precision.

Delivery sends these headers over the exact stored envelope bytes:

| Header | Value |
| --- | --- |
| `Content-Type` | `application/json` |
| `X-Suite-Source` | Sender app id |
| `X-Suite-Timestamp` | Unix seconds, re-signed on each attempt |
| `X-Suite-Delivery-Id` | Envelope event id |
| `X-Suite-Signature` | `baukit-webhook-v1` HMAC SHA-256 signature |
| `X-Suite-Replay` | `1` for replay, omitted for live delivery |

The signature covers the timestamp, delivery id and raw body through
`baukit_core::webhook_signature`. Timestamp skew is at most 300 seconds,
inclusive. Normal event age is at most seven days. Replay relaxes age only;
it still checks source, subject, type, payload and future occurrence, and
forces reward mode `off`. `suite.connection.tested` has an empty payload,
bypasses subscriptions only for an active link, and updates `last_received_at` without
inbox or reward work.

Payload catalogs permit at most 32 keys matching `^[a-z][a-zA-Z0-9]{0,63}$`.
Unknown fields and null values fail. Optional values must be omitted.
Integers reject JSON floats, bounds are inclusive, identifiers use their declared
length and ASCII character rules, UUIDs are lowercase and hyphenated, dates are
canonical civil dates, and timestamps are RFC 3339. `totalXp` must use the JSON
safe integer maximum `9007199254740991`. Products retain their catalog; Baukit's
two-app catalog is only a test sample.

## Routes and failures

Mount the router under the product API base, usually `/api/v1`. Its paths are:

| Method and path | Success | Authentication |
| --- | --- | --- |
| `GET /suite/peers` | 200 | Principal |
| `GET /suite/links` | 200 | Principal |
| `POST /suite/links` | 201 | Principal |
| `POST /suite/links/requests/{id}/complete` | 201 | Principal |
| `GET /suite/links/callback` | 303 | Recorded state |
| `POST /suite/links/exchange` | 201 | Code and PKCE |
| `GET /suite/links/{id}` | 200 | Principal |
| `PATCH /suite/links/{id}` | 200 | Principal |
| `DELETE /suite/links/{id}` | 204 | Principal |
| `POST /suite/links/{id}/test` | 202 | Principal |
| `POST /suite/links/{id}/reenable` | 200 | Principal |
| `POST /suite/links/{id}/replay` | 202, empty body | Principal |
| `GET /suite/links/{id}/deliveries` | 200 | Principal |
| `POST /suite/links/{id}/revoke` | 204 | Signed request |
| `GET /suite/authorizations/preview` | 200 | Principal |
| `POST /suite/authorizations` | 200 | Principal |
| `POST /suite/authorizations/deny` | 200 | Principal |
| `POST /suite/inbound/{id}` | 202 new, 200 duplicate | Signed request |

Principal middleware inserts `SuiteUser { id, suite_subject, display_name }`.
Exempt callback, exchange, inbound and revoke routes from bearer enforcement.
The principal extractor still protects the other routes. Peer responses contain
metadata and links, without product mapping metrics.

Errors use the Baukit error envelope. Protocol codes include `suite_disabled`
(403), `suite_peer_unknown` (400), `suite_return_url_invalid` (400),
`suite_code_invalid` (422, or 400 for an unknown callback state),
`suite_link_account_mismatch` (409), `suite_signature_invalid` (401),
`suite_payload_invalid` (422), `suite_event_id_conflict` (422),
`suite_link_revoked` (410), `suite_link_inactive` (409),
`suite_replay_window` (422), `suite_replay_throttled` (429),
`suite_quota_exceeded` (429, from an applier quota that resets),
`suite_peer_unreachable` (502) and `suite_unavailable` (503).
Envelope validation retains the `event_*` codes from `baukit-events`.
An absent applier returns 503 with `Retry-After: 300` before persisting activity.
Other users' links and requests return 404 `not_found`.

Supply one `Arc<dyn baukit_ratelimit::RateLimitStore>` to the context. Redis
shares budgets across processes. Unknown sources fail before a database read
or limiter key. Known-source misses consume `suite:inbound:unknown:{source}`,
20 per 60 seconds. Known links consume `suite:inbound:{link_id}`, 120 per 60
seconds. Failed exchanges consume `suite:exchange:{client}`; valid exchanges
and authenticated retries remain usable when that bucket is exhausted.
Rate-limited responses include `Retry-After`.

Workers use queue `suite-events`, concurrency 4, ten attempts and backoff
starting at 30 seconds up to six hours. A 2xx succeeds; 401/403 marks
`needs_attention`; 410 revokes. Other nonretryable statuses reject the job.
408, 425, 429, 5xx and transport failures retry. Peer `Retry-After` is capped
at 300 seconds. Twenty consecutive permanent failures disable delivery;
reenable resets the count. Successful delivery resets degraded health.

Metrics use the product's configured prefix, with these suffixes and labels:

| Metric | Labels |
| --- | --- |
| `{prefix}_suite_events_ingested_total` | `outcome` |
| `{prefix}_suite_deliveries_total` | `outcome` |
| `{prefix}_suite_emission_skipped_total` | `reason` |

The cleanup runner counts failures in `suite_cleanup_failures_total`, without
a product prefix or labels.

## Adopt in a product

1. Embed the product's peers and catalog. Construct
   `PeerRegistry::new(own_app: &str, peers_json: &str, settings: PeerRegistrySettings)`
   and `PayloadCatalog::from_json(json: &str)`. App ids are validated strings,
   starting with a lowercase letter, then lowercase letters, digits or `_`,
   with at most 64 bytes. Both public URLs, both peer URLs and at least one
   event intersection are required for an active peer. No active peer means
   standalone mode: peers are hidden and emission enqueues nothing.
   `PeerRegistry::peer_metadata(&self, id: &str) -> Option<&PeerMetadata>` looks
   up the own app and inactive peers as well as active peers. Use its `scheme`
   for product action links, such as `<scheme>://action/start-today`.
   `metadata()` still returns the full embedded list.
2. Embed `SuiteConfig` as `suite` in the product settings and delegate its
   `Validate` implementation. Call `validate_for(own_app, peers_json, cipher)`
   at startup. Active mode requires a credential cipher. URLs require HTTPS;
   `allow_loopback` permits HTTP only for localhost, 127.0.0.1 or ::1.
3. For a product adding suite tables, copy the three suite SQL files after the three
   jobs migrations. Constants are
   `baukit_suite::{SUITE_MIGRATION_SQL, SUITE_RUNTIME_MIGRATION_SQL, SUITE_LOCK_ORDER_MIGRATION_SQL}`.
   Keep applied suite migrations in products that already have these tables.
   For SLS, `0056_suite.sql` corresponds to `001_suite.sql` and
   `0059_suite_runtime.sql` corresponds to `002_suite_runtime.sql`.
   Keep those product files and their SQLx migration history unchanged. Do not
   copy or run `001` and `002` again, and do not replace or renumber applied files.
   Add `003_suite_lock_order.sql` as a new product migration when upgrading
   from 0.10.2. Apply it before deploying the new store code. It replaces the
   failure trigger with job-only accounting records and preserves the counters
   already recorded. SLS keeps `0056`, `0059` and `0061` and adds this migration
   after them. The crate exposes SQL constants; it does not run migrations.
   Compare their schema with the shipped SQL, keeping product owner foreign
   keys. Add a migration only for other differences.
   Owner ids are UUIDs; no owner foreign key is shipped. Add missing owner
   foreign keys with this product migration for each of `suite_links`,
   `suite_link_requests`, `suite_link_codes` and `suite_inbound_events`, replacing
   `owners` and the constraint name for each table:

   ```sql
   ALTER TABLE suite_links ADD CONSTRAINT suite_links_owner_fk FOREIGN KEY (user_id) REFERENCES owners(id) ON DELETE CASCADE;
   ```

4. Implement these product ports with `async_trait`:

   ```rust,ignore
   async fn SuiteEventApplier::apply(
       &self, connection: &mut PgConnection, event: SuiteApplyEvent<'_>,
   ) -> Result<AppliedOutcome, SuiteStoreError>;

   async fn SuiteReplaySource::replay_since(
       &self, connection: &mut PgConnection, owner_id: Uuid, since: NaiveDate,
   ) -> Result<Vec<SuiteEvent>, SuiteStoreError>;

   async fn SuiteIdentitySource::identity(
       &self, owner_id: Uuid,
   ) -> Result<SuiteIdentity, SuiteStoreError>;

   async fn SuiteIdentitySource::identity_in_transaction(
       &self, connection: &mut PgConnection, owner_id: Uuid,
   ) -> Result<SuiteIdentity, SuiteStoreError>;

   async fn SuiteErasureOwnerLookup::lock_owner_in_transaction(
       &self, connection: &mut PgConnection, owner: Uuid,
   ) -> Result<(), sqlx::Error>;
   ```

   Override the default `SuiteEventApplier::lock_owner(connection, owner)`
   hook when product writers lock the owner row. SLS uses
   `SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE`. Lock deleted and
   deactivated owners too and check that state in `apply`. A filter in
   `lock_owner` turns connection tests and duplicate redeliveries for those
   owners into a permanent 404 `suite_rejected`. Ingest calls this hook
   after the suite owner advisory lock and before it locks the link. `apply`
   receives the same transaction with its inbox row already inserted.
   Follow the [crate lock order](https://docs.rs/baukit-suite/latest/baukit_suite/#postgresql-lock-order). Do all reward,
   ledger, progress and downstream event writes on the provided connection.
   Any error rolls back those writes and the inbox. Replay cannot grant a
   reward or return a ledger entry. Use `ValidatedPayload::deserialize<T>()`
   with a product type that denies unknown fields. Its errors convert through
   `?` to `SuiteStoreError::PayloadInvalid`, which returns HTTP 422 with
   `suite_payload_invalid`. Peers do not retry that rejection. The public
   `domain::validation` module exposes field validators for typed parsers.
5. Construct `PostgresSuiteLinkStore::new(pool)` and
   `PostgresSuiteEventOutbox::new(pool, registry, catalog, identities, share_xp, metric_prefix)`.
   Put those stores, a `ReqwestSuitePeerClient::new(allow_loopback)`, limiter,
   replay source and identities into `SuiteDependencies`. Construct
   `SuiteContext::new(registry, catalog, dependencies, SuiteServiceConfig)`.
   The service config carries the cipher, metric prefix, initial replay days
   and global XP switch. Build link, delivery and ingest services, then
   `SuiteModule`. Mount `adapters::http::router(SuiteHttpState { api })` and
   merge `SuiteOpenApi` into the product's OpenAPI document.
6. Emit `SuiteEvent { event_type, natural_key, occurred_at, payload }` at the
   product's domain write points. Before locking or writing product rows, call
   `SuiteEventOutbox::lock_owner_in_transaction(&mut PgConnection, owner_id)`
   on the domain transaction. This serializes the write with ingest and erasure.
   Call
   `SuiteEventOutbox::enqueue_in_transaction(&mut PgConnection, owner_id, &[SuiteEvent])`
   before commit. One job is created per matching active link. Rollback removes
   every job. XP is included only when both global and per-link switches are
   on. Invalid product payloads are skipped and counted; persistence failures
   return errors to the domain caller. An enqueue without the owner lock returns
   `SuiteStoreError::Timeout` if another suite transaction holds it. That error
   leaves the transaction usable. The product can roll back an atomic write or
   commit a best-effort write and recover the event through replay.
7. Register `SuiteJobHandler::new(Arc<dyn SuiteDeliveryRunner>)` with the jobs
   runner. Configure the queue, concurrency and backoff above. Supervise
   `run_hourly_cleanup(service, watch::Receiver<bool>)` alongside it. Cleanup
   runs on UTC hour boundaries. It logs storage errors, increments
   `suite_cleanup_failures_total` and retries at the next boundary. A transaction
   advisory lock skips concurrent cleanup on other replicas.
8. Wrap the product erasure implementation with `PostgresSuiteErasure`, using
   a `SuiteErasureOwnerLookup` for subject-to-owner resolution. Construct
   `SuiteErasureNotificationService` with that adapter and the delivery service.
   Call `erase_with_suite(&ErasureService, &dyn SuiteErasureNotifier, subject, key, &dyn ProductErasure)`.
   It sends best-effort revokes before opening an erasure transaction, with
   a 500 ms bound per link and ignored errors. It covers active, attention,
   disabled, and revoked links with a pending revoke job. Suite jobs and rows
   are then deleted before product rows in the erasure transaction.
   `owner_in_transaction` reads without `FOR UPDATE`. The adapter takes the suite
   owner advisory lock, then calls `lock_owner_in_transaction(connection, owner)`.
   Implement that hook with `SELECT id FROM owners WHERE id=$1 FOR UPDATE`, using
   the product's owner table. The advisory lock waits for domain writes that use
   the outbox owner lock. The row lock also waits for inserts that reference the
   owner. Product erasure then deletes its rows. Preserve
   this lock order when adding owner foreign keys.
9. Add `@baukit/suite-client` to the client. Supply authenticated transport,
   persistent OAuth state, same-tab web redirects and native auth-session
   adapters. Use `SuiteSession.connect(peerApp)` and `handleRedirect(url)`;
   render `SuiteAuthorizeMachine`, `SuiteLinkedMachine` and `ConnectedApps`.
   A successful `ConnectedApps.connect(peerApp)` returns a ready state with
   `completed: { peerApp, requestId }` after refreshing its lists. Pass that
   request id to `SuiteNavigationStore.claimSuiteConnectionAnnouncement`.
   Share this store with `SuiteLinkedMachine` so the native auth session and
   redirect handler announce the connection once. A later `load()` clears
   `completed`. This additive state field keeps the list state and completion
   id together, so SLS can remove its B9 completion workaround.
   Register `suiteMessages`; validate intents with
   `createSuiteNativeIntentValidator(scheme)` and use `openSuitePeer` for
   native-to-web fallback. Product UI supplies event labels and mapping controls.
10. Run the shared fixture checks and product conformance tests. Copy protocol
    vectors only when the product needs a vendored copy. Check `sha256sum -c
    SHA256SUMS` from `fixtures/suite-events/v1`, and compare its protocol fixtures
    with Baukit before changing either side. The sample peers and catalog are
    neutral test data, not the product's deployment metadata.

### Native link query schemes

`@baukit/suite-client/peers` has no runtime dependencies and exports both ESM and
CommonJS. `parsePeersFile(json: unknown): PeerMetadata[]` accepts a parsed
schema-version-1 peers file. It checks the Rust registry's ids, schemes, required
fields, string lists and reward modes, and rejects unknown fields and duplicate
ids. `displayName` must be a string. `peerLinkQuerySchemes(peers, ownAppId)`
returns every other app's scheme, deduplicated and sorted. Products own their
`peers.json`; the package ships no deployment metadata.

A product can use these helpers in `plugins/with-suite-link-queries.cjs`:

```js
const { readFileSync } = require('node:fs');
const { withInfoPlist, withAndroidManifest } = require('@expo/config-plugins');
const { parsePeersFile, peerLinkQuerySchemes } = require('@baukit/suite-client/peers');

module.exports = (config, { ownAppId, peersPath }) => {
  const peers = parsePeersFile(JSON.parse(readFileSync(peersPath, 'utf8')));
  const schemes = peerLinkQuerySchemes(peers, ownAppId);
  config = withInfoPlist(config, (mod) => {
    const existing = mod.modResults.LSApplicationQueriesSchemes ?? [];
    mod.modResults.LSApplicationQueriesSchemes = [...new Set([...existing, ...schemes])].sort();
    return mod;
  });
  return withAndroidManifest(config, (mod) => {
    const queries = (mod.modResults.manifest.queries ??= []);
    const intents = queries.flatMap((query) => query.intent ?? []);
    const missing = schemes.filter((scheme) => !intents.some((intent) =>
      intent.action?.some((action) => action.$['android:name'] === 'android.intent.action.VIEW') &&
      intent.data?.some((data) => data.$['android:scheme'] === scheme)
    ));
    if (missing.length) queries.push({ intent: missing.map((scheme) => ({
      action: [{ $: { 'android:name': 'android.intent.action.VIEW' } }],
      data: [{ $: { 'android:scheme': scheme } }],
    })) });
    return mod;
  });
};
```

Register the plugin in `app.config.cjs` with the product's app id and an absolute
`peersPath`, such as `require.resolve('./suite/peers.json')`. It preserves existing
iOS queries and Android intents. Changing peers requires a new native build.

Configuration keys use the product prefix without renaming:

| `<PREFIX>__SUITE__...` | Default |
| --- | --- |
| `PUBLIC_API_URL`, `PUBLIC_WEB_URL` | unset |
| `PEERS__<APP>__API_URL`, `PEERS__<APP>__WEB_URL` | unset |
| `ALLOW_LOOPBACK` | false |
| `INITIAL_REPLAY_DAYS` | 90, at most 365 |
| `SHARE_XP` | true |
| `IDENTITY_DOMAIN` | unset |
| `IDENTITY_CLAIM` | `suite_sub` |

Apply the ten steps above in every product. The following checklist records the
product work identified in the reference snapshot. Confirm the current migration
head and emit points in each product before adoption.

| Product app id | Config prefix and API base | Product work |
| --- | --- | --- |
| `sololeveling` | `SOLO_LEVELING_SYSTEM`, `/api/v1` | Replace the copied suite module. Keep applied migrations `0056`, `0059` and `0061`, including their owner foreign keys. Keep typed payloads, LifeGraph mappings, rewards and the mapping route. Implement replay from existing domain history. |
| `eigenruhe` | `EIGENRUHE`, `/api/v1` | Replace hub publishing and its connection UI. Emit practice completion and check-ins from REST and sync transactions. Replay those rows. Keep the fitness-tracker connection. |
| `hebkit` | `HEBKIT`, `/api/v1` | Replace the Tiefgang-specific sender and connection route. Emit first-party workouts and sleep, including imported sleep, at existing write points. Update closed job-type dispatch and replay both histories. |
| `leitbild` | `LEITBILD`, `/api/v1` | Add a vault keyring. Run suite delivery independently of the AI worker. Emit new journal entries and completed program runs, excluding backfills. Supply replay and preserve the authorize query across login. |
| `tiefgang` | `TIEFGANG`, `/api/v1` | Replace connection tokens and suite ingest. Keep custom webhooks. Emit terminal sessions and wrap the existing credit-rule engine as the applier. Replay session history. |
| `redemut` | `REDEMUT`, `/v1` | Run the suite worker beside erasure in the API process. Emit newly appended events only. Group review replay by the user's local date. Keep issuer-aware account lookup and same-tab authorize login. |
| `schlauzug` | `SCHLAUZUG`, `/v1` | Run suite jobs in the API process with PostgreSQL. Emit owned solo and room runs in their history transaction; guests do not emit. Replay run history and add same-tab login that preserves the query. |

Every product must add owner foreign keys, erasure inventory coverage, deployment
config, both suite pages, native intent routing where applicable, and its own
applier tests. Host `/suite/authorize` with `Content-Security-Policy:
frame-ancestors 'none'` and `Referrer-Policy: no-referrer`. The headless machine
also refuses framed pages, but the product owns these hosting headers.

## Agreed extraction changes

| Change | Implementation |
| --- | --- |
| a. Generic apps and payloads | Validated string app ids and embedded peer metadata replace the app enum. Catalog field rules preserve integer, bounds, null, enum, date, timestamp, identifier and key checks. The 1,145 payload cases run as shared fixtures. `ValidatedPayload::deserialize<T>()` supports product types. |
| b. UUID owners and erasure | Migrations omit product owner foreign keys. Products add the SQL above. Revokes run before the erasure transaction, bounded to 500 ms per link; the adapter deletes suite jobs and rows by owner inside it. |
| c. Product replay | `SuiteReplaySource` supplies history on the existing connection. The generic outbox only stores and fans out validated events. |
| d. Shared rate limits | `Arc<dyn RateLimitStore>` supplies the three protocol buckets. Unknown sources fail before storage and limiter access. |
| e. Connection test | Every app emits `suite.connection.tested`; protocol fixtures use it. |
| f. Product mapping metadata | Shared peers omit `mappableMetrics`. Products retain their mapping routes. |
| g. Transaction arguments | Transactional ports accept `&mut PgConnection`. The jobs enqueue API accepts the same connection; callers can pass an existing SQLx transaction by dereference. |
| h. Config loading | `SuiteConfig` implements `baukit_config::Validate`, preserves the environment keys and validates active peers, HTTPS, loopback exceptions and standalone mode. |

## Extraction map

The reference is the read-only SLS snapshot at `6a95ac2`. All paths below are
relative to that snapshot; the `solo-leveling-system-` crate prefix is omitted.

| SLS file | Baukit file | Product retains |
| --- | --- | --- |
| `domain/src/suite.rs`, `suite/link_protocol.rs` | `baukit-suite/src/domain/{mod,link_protocol}.rs` | Own app id and embedded peers |
| `domain/src/suite/payloads.rs`, LifeGraph validation | `domain/{payloads,validation}.rs` and validator fixtures | Typed payloads, builders and event catalog |
| `ports/src/suite.rs` | `baukit-suite/src/ports.rs` | Applier, replay source and identity implementations |
| `services/src/suite/{link,delivery,ingest,erasure}.rs` | `baukit-suite/src/services/` | Rewards, mappings, product erasure and domain emits |
| `postgres/src/suite.rs`, `suite/{outbox,rows}.rs` | `adapters/postgres/` | History queries and owner foreign keys |
| `migrations/{0056_suite,0059_suite_runtime}.sql` | `baukit-suite/migrations/{001_suite,002_suite_runtime}.sql` | `0061` mappings and other product tables |
| `api/src/v1/suite.rs` | `adapters/{http,http_support}.rs` | Principal middleware and mapping routes |
| `worker/src/suite.rs`, `bin/src/compose/suite.rs` | `adapters/jobs.rs`, product wiring above | Runner ownership and deployment configuration |
| `fixtures/suite-events/v1/` | `fixtures/suite-events/v1/` | Product catalog, peers and event docs |
| `mobile/app/suite/{authorize,linked}.tsx`, `features/suite/*` | `suite-client/src/{authorize,linked,session,urls,navigation}.ts` | React pages, Expo/browser adapters and routing |
| `features/settings/suite-*`, `localization/messages-suite.ts` | `suite-client/src/{api,connected,messages}.ts` | Product event labels and mapping settings |

Shared protocol tests run against the generic catalog and real Axum router with
Docker PostgreSQL. Product gameplay tests remain with their applier. The test kit's
`suite` feature supplies fixture loading, a scripted receiver and an in-process
second app. No suite capability is added to CLI templates in this extraction.

## SLS bugs fixed

The source paths below refer to snapshot `6a95ac2`.

- `backend/crates/solo-leveling-system-api/src/v1/suite.rs:232` names the
  envelope property `source` in OpenAPI, although the wire envelope uses
  `sourceApp`. The shared schema uses `sourceApp` and has a regression test.
- `backend/crates/solo-leveling-system-services/src/suite/link.rs:514` calls
  replay while holding a transaction. The implementation at
  `backend/crates/solo-leveling-system-postgres/src/suite/outbox.rs:425` acquires
  another pool connection. This blocks a pool of size one. Replay and identity
  queries now use the existing connection; an integration test uses a one-slot
  pool.
- `backend/crates/solo-leveling-system-postgres/src/suite.rs:359` locks a code
  before exchange locks its owner at line 441. Erasure locks the owner in
  `backend/crates/solo-leveling-system-postgres/src/profile_erasure.rs:30` before
  deleting suite rows. These orders can deadlock. The adapter acquires
  the suite owner advisory lock before either a code or request row. A concurrent
  exchange, completion and erasure test verifies that erasure can delete those
  rows while the link operation waits.
- `backend/crates/solo-leveling-system-postgres/src/profile_erasure.rs:125`
  deletes suite jobs before deleting links. Replay and test enqueue methods lock
  only the link at `backend/crates/solo-leveling-system-postgres/src/suite/outbox.rs:337`
  and line 386; disconnect does the same through `suite.rs:427`. They can commit
  another job while erasure waits for that link, leaving an orphan after cleanup.
  Shared link operations take the owner advisory lock first. Router tests race
  replay, test and disconnect with erasure and check that no jobs remain.
