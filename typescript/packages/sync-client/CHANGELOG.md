# @baukit/sync-client

## 0.3.0

### Minor Changes

- 9f0dc94: Add a persisted hybrid logical clock with JavaScript-safe encoding and shared Rust and TypeScript fixtures.
- Release the coordinated baukit 0.3.0 train.
- 4d1f0d3: Add atomic submitted-batch outcome conformance for late accepted and rejected responses.

  Add the browser scheduler environment and an explicit recovery signal callback for waking product-owned retry delays.

- 9b92f9f: Add optional tombstone-horizon and full-resync conformance callbacks and cases.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Add a callback-driven conformance harness for sync implementations.
- Track pull and push attempt and success timestamps, preserve typed transport
  failures, honor `Retry-After`, and validate pull pages and push results.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/sync-client`.
