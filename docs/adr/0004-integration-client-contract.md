# ADR 0004: Integration clients share one dependency-free contract

## Status
Accepted, 2026-09-02.

## Context

Products need the same safeguards around connection settings. Provider errors must not become UI
copy. OAuth callbacks must match a stored state nonce and an allowlisted return endpoint. Web code
must reserve a popup before awaiting a server request, while native code delegates to an auth-session
runner.

Provider names, capabilities, authorization parameters, persistence, and copy still differ by
product. A shared UI component or provider adapter would put those decisions in the toolkit.

## Decision

`@baukit/integrations-client` has no runtime dependencies and holds three pieces:

- a reducer that maps server health and client events to fixed client states and actions;
- an OAuth coordinator whose popup, native runner, redirect handler, storage, nonce source, timeout,
  return URL validator, and clock are injected; and
- an immutable provider registry built only from product-supplied IDs, label keys, capabilities, and
  connection state.

The reducer drops raw provider diagnostics. It retains only a bounded snake_case diagnostic code for
machines. The OAuth coordinator accepts a callback only when its exact origin and path are allowed,
its state matches the stored nonce, and the transaction has not expired.

The provider registry separates product definitions from connection state. A registration may carry
a typed, product-owned connector with functions, hooks or icons, and baukit stores it without reading
it. `withConnectionStates` returns a new registry in registration order and leaves the original
untouched. A provider with no supplied state becomes `disconnected` with no actions. `connection` on a
registration is optional.

The client checks are one half of provider linking. The server half is the single-use,
browser-bound OAuth state in `baukit-integrations` behind its `oauth-state` feature, described in
[ADR 0002](0002-integration-connector-contract.md).

## Enforced by

- `typescript/packages/integrations-client/src/oauth-session.test.ts`: "rejects a return from
  another origin or path" and "returns storage mismatch when the callback state differs".
- `typescript/packages/integrations-client/src/connection-health.test.ts`: "never copies provider
  diagnostics into the user-facing state".
- `typescript/packages/integrations-client/src/provider-registry.test.ts`: "overlays connection
  states without changing the original registry".

## Consequences

Web and native products use the same state transitions and callback checks without importing React,
Expo, or browser globals through the package root. Products still own display text and every
provider-specific branch. A full-page redirect can finish through `handleRedirect` because the
in-flight transaction lives in injected storage.

## History

- 2026-09-02, commit `3d634ce2`: accepted with a registry that held connection state in each
  registration.
- 2026-09-03, commit `1391492d`: registrations carry typed product connectors, and connection state
  moved to `withConnectionStates` overlays.
- 2026-11-27: linked the server-side OAuth state.
