# @baukit/suite-client

Headless suite linking for web and native clients. The package exports a typed
HTTP client, OAuth session handling, authorize and connected-app state, return URL
and native intent guards, peer opening, notices, and English, German and Spanish
messages. It imports no React, Expo or browser globals.

## Client and session

Construct `SuiteClient` with the product's authenticated JSON transport. Prefix
paths with the product API base. Decode Baukit errors into an `Error` with a string
`code`, such as `ApiError` from `@baukit/api-runtime`. Successful replay and delete
requests have empty bodies.

Construct one `SuiteSession` per signed-in session and share it between the native
auth session and redirect handler. Supply `OAuthSessionCoordinator` storage, clock,
nonce, web redirect and native session adapters from `@baukit/integrations-client`.
Web adapters redirect the same tab. Native adapters open an auth session and pass
its result back. Persist OAuth state across a cold launch. The coordinator checks
the return endpoint, nonce and ten-minute expiry before completing the server request.

```ts
import { SuiteClient, ConnectedApps } from '@baukit/suite-client';

// Product adapters supply transport and the configured session instance.
const client = new SuiteClient(transport);
const connectedApps = new ConnectedApps(client, session);
await connectedApps.load();
const result = await connectedApps.connect('beta');
if ((result.type === 'ready' || result.type === 'failed') && result.completed) {
  if (navigation.claimSuiteConnectionAnnouncement(result.completed.requestId)) {
    announceConnected(result.completed.peerApp);
  }
}
```

A successful connection refreshes the lists. `connect()` returns
`completed: { peerApp, requestId }` on the `ready` state, or on the `failed` state
if that refresh fails. A failed state with `completed` means the server connected
the apps; its `code` describes the refresh error. Handle that error separately
from the connection announcement. A later `load()` clears `completed`.

Share one `SuiteNavigationStore` between this caller and `SuiteLinkedMachine`.
Its announcement guard deduplicates the native auth-session result and redirect
notice by request id. The session redeems the code once even if both paths fire.

## Pages and native intents

Use `parseSuiteAuthorizeQuery` before constructing `SuiteAuthorizeMachine`.
Its states cover login, consent, auto-approval, account hint mismatch, denial,
framed-page refusal and native consent that must open the web page. Render those
states in the product UI. `switchSuiteAccount` logs out before starting login.

`SuiteLinkedMachine` keeps the callback URL in memory and asks the product to scrub
codes from history. Wait for identity restoration, then call `restore`. Completion
runs once, refreshes connected apps and creates a notice in `SuiteNavigationStore`.
Scope that store to the product's authenticated session.

`createSuiteNativeIntentValidator(scheme)` permits only the suite authorize and linked
pages. `openSuitePeer` tries the peer scheme before the configured web fallback.
HTTPS is required unless the product explicitly enables loopback HTTP.

Register `suiteMessages` with `@baukit/localization-core`. Products replace
`{product}` and render labels for their own event types. Message key and interpolation
parity is checked across `en`, `de` and `es`.

## Peer metadata at config time

The dependency-free `@baukit/suite-client/peers` subpath has ESM and CommonJS
exports. Expo config plugins can require it without loading the client or its
dependencies. Products own their peers file; none ships in this package.

```js
const { readFileSync } = require('node:fs');
const { parsePeersFile, peerLinkQuerySchemes } = require('@baukit/suite-client/peers');

const peers = parsePeersFile(JSON.parse(readFileSync(peersPath, 'utf8')));
const schemes = peerLinkQuerySchemes(peers, ownAppId);
```

`parsePeersFile(json: unknown): PeerMetadata[]` validates the schema-version-1
file with the Rust registry rules. `peerLinkQuerySchemes(peers: readonly
PeerMetadata[], ownAppId: string): string[]` excludes the own app by id, then
deduplicates and sorts schemes. Use the result for iOS
`LSApplicationQueriesSchemes` and Android `<queries>`. The adoption guide has a
config plugin that preserves existing entries.

See [suite adoption](../../../docs/platform/suite-events.md) for server wiring
and the protocol contract. This package adds no CLI template capability.
