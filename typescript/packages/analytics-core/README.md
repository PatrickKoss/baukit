# `@baukit/analytics-core`

Provider-neutral, privacy-first product analytics for Baukit TypeScript applications. The package
defines the port and the safety policy; PostHog support belongs in the separate web and native
adapter packages. It has no runtime dependencies.

## Why a port, not a provider wrapper

Product code should own event meaning and compile-time shape without importing a provider SDK. The
core accepts a product event union, applies consent and privacy controls, and sends neutral command
envelopes to a `Transport`. A provider adapter only translates those already-sanitized envelopes.

The required order for captured properties is:

1. Keep only keys in that event's allowlist.
2. Scrub blocked keys and sensitive string patterns.
3. Stamp the required common context and identity.
4. Enqueue the sanitized envelope for transport.

## Typed setup

```ts
import {
  AnalyticsClient,
  type AnalyticsEvent,
  type EventAllowlist,
  type Transport,
} from '@baukit/analytics-core';

type ProductEvent =
  | {
      name: 'onboarding_started';
      properties: { source: 'organic' | 'invite' };
    }
  | {
      name: 'onboarding_completed';
      properties: { duration_seconds: number };
    };

const allowlist = {
  onboarding_started: ['source'],
  onboarding_completed: ['duration_seconds'],
} as const satisfies EventAllowlist<ProductEvent>;

declare const transport: Transport<ProductEvent>; // for example, a PostHog adapter

const analytics = new AnalyticsClient<ProductEvent>({
  context: {
    schema_version: 1,
    app: 'architecture-health',
    app_version: '1.0.0',
    platform: 'web',
    environment: 'production',
    locale: 'de-DE',
  },
  allowlist,
  transport,
});

analytics.capture({
  name: 'onboarding_started',
  properties: { source: 'organic' },
});
```

`AnalyticsClientOptions.context` is required, so constructing a client without
`schema_version`, `app`, `app_version`, `platform`, `environment`, or `locale` does not type-check.
Adding a name to `ProductEvent` also makes the matching allowlist entry required. Runtime input can
still contain excess keys (for example, after an unsafe cast or across a JavaScript boundary), so
the allowlist drops them before transport.

Event names are a product-owned convention: stable, past-tense `snake_case`, such as
`onboarding_completed`. Encoding English past tense in template-literal types would reject valid
irregular forms and create false confidence. Instead, the client uses a conservative development
warning (snake case plus a past-tense heuristic); it never changes runtime delivery. Tracking-plan
purpose and ownership remain product responsibilities.

## Consent

Consent starts as `unknown`. Captures, identify calls, and alias calls made while consent is
`unknown` or `denied` are dropped immediately and are never buffered for later opt-in.

```ts
analytics.setConsent('granted'); // persist opt-in, then analytics may run
analytics.capture({
  name: 'onboarding_completed',
  properties: { duration_seconds: 42 },
});

analytics.setConsent('denied'); // persist withdrawal and discard every unsent command
```

`setConsent` is not itself an analytics event. Consent is read synchronously during construction,
before the first capture is possible. `InMemoryAnalyticsStorage` is the dependency-free default;
products should inject a small synchronous `AnalyticsStorage` backed by device storage. If the
platform's native storage API is asynchronous, hydrate a synchronous application-owned cache
before constructing the client. Storage failures fail privacy-safe: unreadable consent becomes
`unknown`, and analytics methods do not throw because persistence diagnostics fail.

Withdrawal clears commands that have not been handed to the transport. A batch already in flight
cannot be recalled, which is a transport boundary rather than hidden buffering.

## Identity

The client creates and persists an anonymous UUID with `globalThis.crypto.randomUUID()`. Runtimes
without it, including some React Native configurations, must inject a cryptographically secure
`uuidFactory`; there is intentionally no `Math.random()` fallback.

```ts
analytics.setConsent('granted');
const anonymousId = analytics.anonymousId;

analytics.identify(internalUserUuid, { plan: 'free' });
analytics.alias(anonymousId, internalUserUuid); // accepted once for this anonymous identity

analytics.reset(); // logout: clear known identity and alias guard, rotate anonymous UUID
```

Only UUID-shaped user IDs are accepted. This cannot prove which system issued a UUID, so products
must pass their internal user UUID—not an email, name, or provider subject. Switching directly
between known users is rejected; call `reset()` first. The alias guard is persisted and admits at
most one alias command for an anonymous identity, including across client reconstruction. Traits
are scrubbed with the same blocked-key and sensitive-value rules as event properties.

## Scrubbing

The default key blocklist is `email`, `name`, `token`, `password`, `authorization`, `cookie`,
`phone`, and `address`. Matching is case/separator insensitive and deliberately conservative, so
keys such as `displayName` and `auth_token` are redacted. Products can add terms through
`blockedKeys`. Email-shaped strings, JWT-shaped strings, and long hex/base64-like strings become
`"[redacted]"`. Nested arrays and plain objects are traversed without mutating the input.

The scrubber is only a last line of defense. Event properties must remain bounded metadata: never
health or food values, conversation or prompt content, free text, precise location, credentials,
or raw identifying URLs.

## Buffering and failures

Accepted commands enter an in-memory queue with these defaults:

- maximum 100 waiting commands;
- flush at 20 commands or after 5 seconds;
- oldest waiting command dropped on overflow;
- two retries after the initial transport attempt, with short exponential delays;
- a failed batch dropped after the retry cap.

`capture`, `identify`, `alias`, `reset`, and `setConsent` are synchronous and never wait for a
provider. Size-triggered delivery is scheduled in a microtask; interval delivery uses an unref'd
timer where the runtime supports it. Transport rejection and synchronous transport exceptions are
caught internally and never escape into the user journey. `onTransportFailure` is available for
bounded diagnostics, and even exceptions from that callback are contained.

`flush()` is an explicit best-effort boundary that always resolves, useful for tests and app
lifecycle hooks. `dispose()` performs a best-effort flush and clears its timer. Queues are not
persisted: application restarts do not resurrect analytics commands, and unknown/denied events can
never appear after later consent.

`NoopTransport` safely discards commands. `InMemoryTransport` retains inspectable envelopes for
tests. PostHog web and React Native packages plug in by implementing `Transport<E>` and translating
the neutral `capture`, `identify`, `alias`, and `reset` envelope variants; product code and privacy
policy stay unchanged.
