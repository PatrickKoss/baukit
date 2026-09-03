# `@baukit/integrations-client`

`@baukit/integrations-client` provides provider-neutral client behavior for integration settings:

- a connection reducer with fixed states and actions;
- an OAuth coordinator with persisted state checks and strict return URL validation; and
- a typed provider registry built from product data.

The package has no runtime dependencies. Its root entry does not import React, Expo, or browser
globals.

## Connection state

Map the server's connection health before applying client events:

```ts
import { connectionStateFromServer, reduceConnectionState } from '@baukit/integrations-client';

const state = connectionStateFromServer({
  state: 'needs_reconnect',
  diagnosticCode: 'authorization_revoked',
  providerDiagnostic: upstreamError,
});
const connecting = reduceConnectionState(state, { type: 'connect_requested' });
```

`providerDiagnostic` is untrusted input and is always dropped. `diagnostic.code` accepts only a
bounded snake_case code. Products map `status` and `availableActions` to their own localized copy.
They must not render the diagnostic code.

## OAuth sessions

Create one coordinator with product-owned ports for storage, timers, nonce creation, and the web or
native authorization runner. The coordinator saves the nonce before it opens the authorization URL.
It returns `success`, `cancelled`, `timed_out`, `invalid_return`, or `storage_mismatch`.

```ts
import { OAuthSessionCoordinator, createReturnUrlValidator } from '@baukit/integrations-client';

const coordinator = new OAuthSessionCoordinator({
  storage,
  clock,
  timeoutMs: 15_000,
  createStateNonce,
  validateReturnUrl: createReturnUrlValidator({
    origin: 'https://app.example',
    paths: ['/settings/integrations/callback'],
  }),
  browser,
  native,
  redirect,
});

const outcome = await coordinator.authorize({
  platform: 'web',
  returnUrl: 'https://app.example/settings/integrations/callback',
  createAuthorizationUrl: startConnection,
});
```

On web, `browser.openPlaceholder` runs before any asynchronous work so a click can reserve the
popup. If it returns `null`, the coordinator uses the injected redirect handler. After a full-page
return, call `handleRedirect` with the current URL. Native products supply `native.run` and
`native.cancel` using their auth-session library.

The validator requires the exact configured origin and one exact allowlisted path. Callback checks
reject lookalike origins, path prefixes, credentials in URLs, malformed URLs, and duplicate
parameters.

## Provider registry

Register static product data once. The connector type belongs to the product, so it can hold an
OAuth starter, product hooks, icons, or other UI-specific values without adding those dependencies
to Baukit:

```ts
import { createProviderRegistry } from '@baukit/integrations-client';

const providerDefinitions = createProviderRegistry([
  {
    id: 'calendar',
    labelKey: 'integrations.calendar',
    capabilities: ['read_events'],
    connector: {
      startOAuth: startCalendarOAuth,
      useConnection: useCalendarConnection,
      icon: CalendarIcon,
    },
  },
  {
    id: 'storage',
    labelKey: 'integrations.storage',
    capabilities: ['read_files'],
    connector: {
      startOAuth: startStorageOAuth,
      useConnection: useStorageConnection,
      icon: StorageIcon,
    },
  },
] as const);

const providers = providerDefinitions.withConnectionStates(
  new Map([
    ['calendar', calendarConnectionState],
    ['storage', storageConnectionState],
  ]),
);

await providers.get('calendar')?.connector?.startOAuth();
```

`list()` keeps registration order. `get()` returns the typed connector and returns `undefined` for an
unknown ID. `withConnectionStates()` accepts a read-only map or partial record and returns a new
registry; it does not change `providerDefinitions`. A provider omitted from the state collection has
state `disconnected` and no available actions. Passing `connection` in a registration still works
for callers using the 0.2.0 API.

The registry copies capability and action arrays and rejects duplicate IDs. It keeps connector
objects by reference so hooks, functions, and component identities remain stable. Persistence,
provider OAuth parameters, identity, copy, and policy stay in the product.
