# `@baukit/analytics-posthog-native`

PostHog React Native transport for `@baukit/analytics-core`. It translates only envelopes that
have already passed the core's consent gate, event allowlist, and scrubber.

## Self-hosted setup

Install the core and React Native SDK alongside the adapter:

```sh
pnpm add @baukit/analytics-core @baukit/analytics-posthog-native posthog-react-native
```

Use the configured factory with an explicit self-hosted URL. There is deliberately no PostHog
Cloud default:

```ts
import { AnalyticsClient } from '@baukit/analytics-core';
import { createPostHogNativeTransport } from '@baukit/analytics-posthog-native';

const transport = createPostHogNativeTransport({
  apiKey: 'phc_project_key',
  apiHost: 'https://posthog.analytics.example.com',
});

const analytics = new AnalyticsClient({
  context,
  allowlist,
  transport,
  uuidFactory: secureReactNativeUuidFactory,
});
```

SDK initialization is lazy: `posthog-react-native` is loaded and constructed only when the core
sends its first consent-approved batch. Its initial distinct ID is bootstrapped from the core
envelope. App lifecycle capture, push capture, default person properties, error autocapture,
session replay, surveys, and feature-flag preloading are disabled. Person profiles are limited to
identified users.

An application that already owns a configured client can wrap it instead:

```ts
import { PostHogNativeTransport } from '@baukit/analytics-posthog-native';

const transport = new PostHogNativeTransport(posthog);
```

When wrapping an instance, the application remains responsible for initializing it only after
consent and applying equivalent privacy settings.

The adapter exports a minimal structural `PostHogNativeClient` interface for mocks and wrappers,
while its initializer options compile against `posthog-react-native`'s published types. This keeps
unit tests runnable without installing or booting a React Native runtime.

## Persisted identity storage

`AnalyticsClient` reads storage synchronously, while React Native storage is asynchronous. The
`/storage` subpath bridges the two. `HydratedAnalyticsStorage.load` reads the listed keys once,
serves later reads from memory, and writes those keys through without waiting. Other keys stay in
memory. The subpath does not import `posthog-react-native`, so apps with another transport can use
it without the SDK.

```ts
import AsyncStorage from '@react-native-async-storage/async-storage';
import { AnalyticsClient, analyticsStorageKeys } from '@baukit/analytics-core';
import { HydratedAnalyticsStorage } from '@baukit/analytics-posthog-native/storage';

const keys = analyticsStorageKeys(storageKeyPrefix);
const storage = await HydratedAnalyticsStorage.load({
  persistence: AsyncStorage,
  persistentKeys: [keys.anonymousId, keys.userId, keys.aliasedUserId],
});
const analytics = new AnalyticsClient({ context, allowlist, storage, storageKeyPrefix });
```

Include `keys.consent` only when analytics consent has no other durable home. A key that fails to
read starts absent, and the other keys still load. A failed write keeps the in-memory value and is
not reported. The storage holds only what the client writes: the consent state and pseudonymous
UUIDs.

`posthog-react-native` is an optional peer. Install it only when you use the transport.

## Mapping

- `capture` flattens the core-stamped context and already-scrubbed event properties, preserving the
  core capture time as the PostHog timestamp.
- `identify` passes the internal user UUID and the core's already-scrubbed traits. Those traits are
  the only values used for PostHog person-property `$set` behavior.
- `alias` passes the core's known user UUID to the React Native SDK's alias method.
- `reset` resets the PostHog identity after the core has rotated its own anonymous UUID.

## Deliberate omissions

This package does not decide consent, generate or validate product identity, define events,
allowlist properties, scrub PII, buffer commands, enable automatic capture, record sessions, or add
provider-specific event properties. Those policy decisions stay in `@baukit/analytics-core` (or,
for provider project retention and deletion, in operations).
