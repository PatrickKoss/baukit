# Product analytics privacy and identity contract

**Status:** Adopted.
**Home:** this repository, alongside `@baukit/analytics-core`.

Applies to every product using the shared analytics packages. Self-hosted PostHog on a dedicated server is the default provider behind the port; nothing in this contract may depend on PostHog specifics.

## 1. Events

- Names are past tense, snake_case (`onboarding_completed`).
- Every event is declared in the product's typed event union; untyped capture calls must not compile.
- Every event has a documented purpose and owner in the product's tracking plan.
- Every event carries `schema_version` and the common context: `app`, `app_version`, `platform`, `environment`, `locale`.
- Properties are allowlisted per event; unknown properties are dropped before transport, not sent.

## 2. Forbidden content

Never captured as event properties, enforced by type and by test: health and fitness values, food data, conversation or prompt content, free-text user input, email addresses, names, tokens or credentials, precise location, and raw URLs containing identifiers.

Bounded aggregates and enums about these domains are allowed: `meal_logged` with `meal_type: "dinner"` is fine; the meal contents are not.

## 3. Consent

- States: `unknown`, `granted`, `denied`. The default is `unknown`.
- In `unknown` and `denied`, events are dropped, not buffered for later delivery.
- Jurisdiction baseline is GDPR (products operated from Germany): analytics runs only after explicit opt-in, withdrawing consent is as easy as granting it, and consent changes are not themselves analytics events.
- Consent state is stored per device and re-evaluated on application start.
- On withdrawal, both the core buffer and any provider-owned persisted or retry queue are purged before they can flush later.

## 4. Identity

- The anonymous ID is a random client-generated UUID, never an authentication identifier.
- On login or signup, `identify` uses the internal user UUID (never the provider subject, email, or name), and `alias` links the anonymous history exactly once.
- On logout, `reset` clears identity and generates a fresh anonymous ID.
- Analytics identifiers are never used for authentication and never sent to the backend as auth context.

## 5. Scrubber (last line of defense)

- Key blocklist: `email`, `name`, `token`, `password`, `authorization`, `cookie`, `phone`, `address`, plus product-specific additions.
- Value patterns: email addresses, JWT-shaped strings, and long hex/base64 secrets are replaced with `[redacted]`.
- The scrubber runs after the typed allowlist and before transport. It is a safety net; the types are the primary control.
- Scrubber unit tests are mandatory in `analytics-core` and in every product's event package.

## 6. Client versus server events

- Clients capture interface interactions (`checkout_started`).
- The backend captures authoritative outcomes (`subscription_activated`), written through a transactional outbox in the same database transaction as the state change and delivered asynchronously. A user request never blocks on the analytics provider.

## 7. Retention and deletion

- PostHog project retention target: 12 months of raw events; reviewed yearly.
- Account deletion triggers deletion of the person and their events in PostHog (deletion API) within 30 days; each product's deletion runbook documents the step.
- The export path (provider-neutral port plus PostHog export) is verified before any product depends on analytics for a marketed feature.

## 8. Conformance

- `analytics-core` ships no-op and in-memory transports; its tests assert consent gating, identity transitions, allowlisting, and scrubbing.
- Adding an event requires: the type, a purpose/owner line in the tracking plan, the allowlist entry, and passing privacy tests. The `add-product-event` skill automates this sequence.

## 9. Transport requirements

- The provider-neutral `Transport` may implement `clearPending?(): Promise<void> | void`. `analytics-core` invokes it once whenever consent transitions into `denied`; synchronous throws and rejected promises are contained like send failures. Transports without the capability remain valid.
- Provider adapters with their own queues must implement the hook by opting the SDK out first and then purging every pending persisted, batch, and retry queue the SDK exposes. A later consent-granted send may opt the provider back in without emitting a provider opt-in analytics event.
- `reset()` is an ordered identity command, not consent withdrawal. It rotates the core anonymous identity and is delivered through `send`; it does not invoke `clearPending` or discard earlier consented commands.
