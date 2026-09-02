# ADR 0004: Integration client contract

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

Add `@baukit/integrations-client` with three dependency-free pieces:

- a reducer that maps server health and client events to fixed client states and actions;
- an OAuth coordinator whose popup, native runner, redirect handler, storage, nonce source, timeout,
  return URL validator, and clock are injected; and
- an immutable provider registry built only from product-supplied IDs, label keys, capabilities, and
  connection state.

The reducer drops raw provider diagnostics. It retains only a bounded snake_case diagnostic code for
machines. The OAuth coordinator accepts a callback only when its exact origin and path are allowed,
its state matches the stored nonce, and the transaction has not expired.

## Consequences

Web and native products use the same state transitions and callback checks without importing React,
Expo, or browser globals through the package root. Products still own display text and every
provider-specific branch. A full-page redirect can finish through `handleRedirect` because the
in-flight transaction lives in injected storage.
