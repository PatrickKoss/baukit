# `@baukit/analytics-posthog-web`

PostHog browser transport for `@baukit/analytics-core`. It translates only envelopes that have
already passed the core's consent gate, event allowlist, and scrubber.

## Self-hosted setup

Install the core and browser SDK alongside the adapter:

```sh
pnpm add @baukit/analytics-core @baukit/analytics-posthog-web posthog-js
```

Use the configured factory with an explicit self-hosted URL. There is deliberately no PostHog
Cloud default:

```ts
import { AnalyticsClient } from '@baukit/analytics-core';
import { createPostHogWebTransport } from '@baukit/analytics-posthog-web';

const transport = createPostHogWebTransport({
  apiKey: 'phc_project_key',
  apiHost: 'https://posthog.analytics.example.com',
});

const analytics = new AnalyticsClient({
  context,
  allowlist,
  transport,
});
```

SDK initialization is lazy: the configured PostHog instance is created only when the core sends
its first consent-approved batch. Its initial distinct ID is bootstrapped from the core envelope.
Autocapture, pageview/pageleave capture, exception capture, session recording, surveys, and the
initial feature-flag request are disabled. Person profiles are limited to identified users.
Automatic campaign and referrer person properties are also disabled.

An application that already owns a configured `posthog-js` instance can wrap it instead:

```ts
import posthog from 'posthog-js';
import { PostHogWebTransport } from '@baukit/analytics-posthog-web';

const transport = new PostHogWebTransport(posthog);
```

When wrapping an instance, the application remains responsible for initializing it only after
consent and applying equivalent privacy settings.

## Mapping

- `capture` flattens the core-stamped context and already-scrubbed event properties, preserving the
  core capture time as the PostHog timestamp.
- `identify` passes the internal user UUID and the core's already-scrubbed traits. Those traits are
  the only values used for PostHog person-property `$set` behavior.
- `alias` passes the known user UUID as the alias and the core anonymous UUID as the original ID.
- `reset` resets the PostHog identity after the core has rotated its own anonymous UUID.

## Deliberate omissions

This package does not decide consent, generate or validate product identity, define events,
allowlist properties, scrub PII, buffer commands, enable automatic capture, record sessions, or add
provider-specific event properties. Those policy decisions stay in `@baukit/analytics-core` (or,
for provider project retention and deletion, in operations).
