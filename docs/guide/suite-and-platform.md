# Join a suite and receive hub events

At the end of this page you know what a suite, a hub and a link are, and you have chosen which parts your product needs. You can link your product to a peer app for one user, receive the hub's signed events, read entitlements, and sell store plans on mobile. The page is for product developers whose product runs next to other products of the same owner. A product that stands alone can skip it.

## Concepts

A **suite** is a set of separate products that one user can connect. Each product keeps its own accounts, database and deploy. Nothing is shared at runtime except signed HTTP calls.

A **link** connects one user's account in product A to the same person's account in product B. The user starts it from product A, approves it on product B, and each side stores the link with a shared HMAC secret. After that, a domain write in A can send a signed event to B. For example, finishing a workout in one product can grant a reward in another. Links belong to users. Either side can revoke a link.

The **hub** is one central service that owns accounts across products, plans, billing and organizations. It does not link users. It tells each consumer product what a user is entitled to, and it sends service-level events such as `entitlements.changed` or `organization.updated`. A consumer product receives those events on the **platform receiver** and reads entitlements through signed **tickets**.

Links and the platform receiver are independent. A product can use either one, both or neither.

| You want | Use | `baukit-suite` features |
| --- | --- | --- |
| Peer types, catalog and payload validation in a domain or ports crate | domain only | `default-features = false` |
| User links and signed peer events, with the supplied router, worker and delivery client | suite links | `http`, `jobs`, `delivery` |
| Suite links with your own peer HTTP client | suite links | `runtime` |
| Hub events, entitlements and tickets | platform receiver | `platform`, `postgres` |

The feature table in the [baukit-suite README](../../rust/crates/baukit-suite/README.md#features) lists what each feature adds.

## Generate the platform receiver

The platform receiver is the part a template generates. Run:

```sh
baukit new my-product --backend --platform
```

Add `--worker` to get a daily job that purges old receipts. Without it the product gets a `platform-purge` binary for an external scheduler. Add `--auth oidc` to get account erasure wired to the entitlement cache.

The generated product has these pieces, rendered from `templates/platform/`:

- `backend/crates/<app>-ports/src/platform.rs` holds the `PlatformEvents` port your product implements.
- `backend/crates/<app>-bin/src/platform.rs` holds `compose`, `compose_with` and `purge_receipts`.
- `backend/crates/<app>-api/src/platform.rs` and `<app>-postgres/src/platform.rs` hold the adapters.
- `backend/tests/platform_integration.rs` tests the flow against PostgreSQL.
- `docs/platform.md` lists every runtime setting.

`baukit.toml` records `capabilities.platform = true` and the product-owned migration under `[platform].migration`. The [manifest guide](manifest.md) covers those keys.

## Run standalone first

With no hub settings, `platform::compose` returns an `Entitlements` service that answers with the standalone defaults. The default tier is `free` and the feature list is empty. No inbound route is mounted. Run it locally with the generated Makefile:

```sh
make setup
make db-up
make migrate
make run
```

Handlers read entitlements through `Extension<Entitlements>`:

- `entitlements.current(subject, now)` returns the snapshot.
- `has_entitlement(subject, feature, now)` checks one feature key.
- `has_tier(subject, tier, now)` checks the tier.

Your product decides what each feature and tier means.

## Connect to the hub

Set the hub settings in runtime config. All keys start with the app prefix, as `templates/platform/docs/platform.md` lists:

- `PLATFORM__HUB_ISSUER` and `PLATFORM__JWKS_URL` for ticket verification.
- `PLATFORM__PRODUCT_ID`, the audience the hub puts in tickets for this product.
- `PLATFORM__SOURCE_APP` and the optional `PLATFORM__EVENT_TYPE_PREFIX`, which the receiver checks on each envelope.
- `PLATFORM__RECEIVER_SECRET` and `PLATFORM__PREVIOUS_RECEIVER_SECRET` for the HMAC.
- `PLATFORM_HUB__TICKET_URL` and `PLATFORM_HUB__SERVICE_TOKEN` for pulling tickets.

Partial hub configuration fails validation at startup. In hub mode the API merges `POST /platform/inbound` outside user authentication. The HMAC over the raw body is the authentication. Like every route, it sits inside the rate limiter.

When an `entitlements.changed` event arrives, `EntitlementEventHandler` pulls a fresh snapshot from the hub before it opens the receipt transaction, then stores the snapshot inside it. Every other event goes to your `PlatformEvents` handler, which gets an `EventContext`. Use `context.postgres()` for writes that must commit with the receipt.

Reads pull from the hub on a cache miss or expiry. A failed pull returns an error. It never falls back to an expired paid grant. The hub does not backfill, so pull current state for active subjects when you first enable the receiver.

The `HubTicketSource` adapter in the generated API posts `{suiteSubject, productId}` to the ticket URL and verifies the returned ticket against the hub JWKS. Keep the service token on the server. It never goes into a browser or an app.

On Kubernetes, the chart reads the receiver secret from `platform.receiverSecretRef` with the keys `name`, `key` and `previousKey` in `deploy/chart/baukit-app/values.yaml`. The defaults leave `name` empty, so a standalone deployment needs no Secret. Set `previousKey` only while rotating.

## Keep receipts and handle erasure

The inbox stores a receipt per event to drop duplicates. Keep receipts for at least 30 days. The worker flavor registers a retention job on a daily `FixedUtcInterval` slot. Other flavors run:

```sh
cargo run --manifest-path backend/Cargo.toml -p my-product-bin --bin platform-purge
```

That command comes from `templates/platform/docs/platform.md`. Both paths call `PostgresPlatformInbox::purge_received_before(cutoff, batch)`.

When a user is erased, call `PostgresEntitlementCache::forget_in_transaction(subject, connection)` in the erasure transaction. With `--auth oidc` the generated `PostgresProfileErasure` does it for you. A late `entitlements.changed` can recreate the row, so filter events for unknown subjects before `EntitlementEventHandler` prepares them. A filter inside `PlatformEvents::handle` runs too late, because entitlement events bypass that handler. The [privacy guide](privacy.md) covers erasure.

## Verify tickets on the client

A ticket is an Ed25519-signed JWT with the subject, product audience, tier and features. Apps can verify one offline. This snippet is from `docs/platform/platform-receiver.md`:

```ts
import { verifyTicket } from '@baukit/suite-client/entitlements';

const claims = verifyTicket(ticket, trustedJwks, {
  issuer: configuredHubIssuer,
  audience: productId,
  now: Math.floor(Date.now() / 1000),
});
if (claims.sub !== currentSuiteSubject) throw new Error('ticket subject mismatch');
```

Always compare `sub` with the current subject. The server side has the same checks in `TicketVerifier::verify`. The receiver contract lists the lifetime, key cache and error codes.

## Link two products

No template generates suite links. You add them to an existing backend with the `baukit-suite` features `http`, `jobs` and `delivery`. The [suite events contract](../platform/suite-events.md) has the adoption checklist. The short version:

1. Embed your `peers.json` and `catalog.json`. The peers file names your own app id, its public URLs and its native `scheme`, plus each peer. The catalog lists which event types you send and accept, with payload rules.
2. Embed `SuiteConfig` as `suite` in your settings and call `validate_for` at startup. Active peers need a credential cipher from `baukit-credential-vault`.
3. Copy the suite SQL constants after the `baukit-jobs` migrations: `SUITE_MIGRATION_SQL`, `SUITE_RUNTIME_MIGRATION_SQL`, `SUITE_LOCK_ORDER_MIGRATION_SQL` and `SUITE_RECORD_IDS_MIGRATION_SQL`. Add owner foreign keys yourself.
4. Implement `SuiteEventApplier`, `SuiteReplaySource` and `SuiteIdentitySource`.
5. Compose the suite and mount its router under `/api/v1`.

Step 5 looks like this in the [baukit-suite README](../../rust/crates/baukit-suite/README.md#product-composition). The block is not compiled, so treat it as a shape:

```rust,ignore
let cipher = config.credential_vault.cipher()?;
let settings = SuiteSettings::new(APP_ID, PEERS_JSON, CATALOG_JSON, &config.suite, cipher)?;
let suite = PostgresSuite::builder(pool.clone(), settings, limiter)
    .identity(Arc::new(ProductSuiteIdentitySource::new(pool.clone())))
    .replay(Arc::new(ProductSuiteReplaySource))
    .applier(Arc::new(ProductSuiteEventApplier))
    .erasure_lookup(
        Arc::new(ProductSuiteErasureOwnerLookup::new(pool.clone())),
        Arc::new(PostgresProfileErasure),
    )
    .build()?;
let routes = Router::new().nest("/api/v1", suite.router(suite.api()));
let worker = suite.worker(Arc::new(job_store))?;
```

With no active peer, the suite runs in standalone mode. Peers are hidden and emitting an event enqueues nothing.

### How a link is made

The protocol is an OAuth-style code flow with PKCE S256:

1. The user's client calls `POST /suite/links` on product A with a return URL and a client nonce. A stores a hashed state and a sealed PKCE verifier and returns product B's authorize URL.
2. B's authorize page checks the account, asks for consent and issues a code that lives one minute.
3. B redirects to A's `GET /suite/links/callback`, which relays the code to the recorded return URL. The return URL must be `<public web URL>/suite/linked` or `<scheme>://suite/linked`.
4. The client calls `POST /suite/links/requests/{id}/complete`. A exchanges the code with B and commits the link.

A link request lives ten minutes. Only the user who started it can complete it.

### Send and receive events

Your domain write calls `SuiteEventOutbox::lock_owner_in_transaction`, then emits a `SuiteEvent` with `enqueue_in_transaction`, then commits. The `baukit-jobs` worker delivers it with `X-Suite-*` headers and a `baukit-webhook-v1` HMAC signature over the timestamp, delivery id and raw body. The receiver checks the signature, a 300 second skew window, the catalog payload rules and the event id, then calls your `SuiteEventApplier::apply` in the same transaction. The applier returns `AppliedOutcome` with `Granted`, `NoRule` or `Capped`.

Suite subjects have the form `{identity_domain}|{claim}`. Build them from the authenticated principal with `suite_subject`. Never accept one from a request body.

### The client side

`@baukit/suite-client` runs the link flow in web and native apps. Construct `SuiteClient` with your authenticated transport and one `SuiteSession` per signed-in session, then use `ConnectedApps` to list and connect peers. The snippet in its README shows `connectedApps.connect('beta')`. The OAuth redirect uses `OAuthSessionCoordinator` from `@baukit/integrations-client`, the same one the [integrations guide](integrations.md) uses.

## Sell plans on mobile

`@baukit/purchases-core` runs a store purchase flow in a mobile app. `PurchaseFlow` takes a `PurchasesPort` and a backend from `createAccountPlanBackend`. It never trusts the store receipt for access. After a purchase, the flow waits for the hub's refreshed ticket to show the entitlement. `@baukit/purchases-revenuecat` implements the port on `react-native-purchases`. Use `createNoopPurchasesPort()` for builds without a store key.

The backend side of the account plan is the "Serve the account plan" section of the [platform receiver contract](../platform/platform-receiver.md#serve-the-account-plan).

## Reference

- [baukit-suite README](../../rust/crates/baukit-suite/README.md)
- [Suite events contract](../platform/suite-events.md)
- [Platform receiver contract](../platform/platform-receiver.md)
- [Template doc: platform receiver](../../templates/platform/docs/platform.md)
- [@baukit/suite-client README](../../typescript/packages/suite-client/README.md)
- [@baukit/purchases-core README](../../typescript/packages/purchases-core/README.md)
- [@baukit/purchases-revenuecat README](../../typescript/packages/purchases-revenuecat/README.md)
- [@baukit/integrations-client README](../../typescript/packages/integrations-client/README.md)
- [ADR 0024: suite linking and signed delivery](../adr/0024-suite-linking-and-signed-delivery.md)
- [ADR 0003: event envelope](../adr/0003-event-envelope.md)
- [ADR 0020: durable jobs, outbox and leases](../adr/0020-durable-jobs-outbox-and-leases.md)
