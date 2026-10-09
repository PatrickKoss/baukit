# Changelog

## [Unreleased]

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.7
  - @baukit/localization-core@0.10.7

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.6
  - @baukit/localization-core@0.10.6

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.5
  - @baukit/localization-core@0.10.5

## 0.10.4

- Keep the peer and completed request id on the failed state when a connection succeeds but its list refresh fails. Clear the completion on a later list load.

### Patch Changes

- Release the coordinated baukit 0.10.4 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.4
  - @baukit/localization-core@0.10.4

## 0.10.3

- Keep the peer and completed request id on the ready state returned by `ConnectedApps.connect`. Clear that completion on a later list load.
- Test one code redemption and one announcement when native completion and the redirect handler both fire.

### Patch Changes

- Release the coordinated baukit 0.10.3 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.3
  - @baukit/localization-core@0.10.3

## 0.10.2

- Read the npm 12 `npm pack --json` format in the publish check. The 0.10.1 npm publish stopped on it, so 0.10.2 is the first release with installable peers.

### Patch Changes

- Release the coordinated baukit 0.10.2 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.2
  - @baukit/localization-core@0.10.2

## 0.10.1

- Declare sibling Baukit packages as versioned peers so npm consumers can install the package. Keep workspace links in development dependencies.
- Check packed dependency specifiers before publishing and in CI.

### Patch Changes

- Release the coordinated baukit 0.10.1 train.
- Updated dependencies
  - @baukit/integrations-client@0.10.1
  - @baukit/localization-core@0.10.1

## 0.10.0

- Add the dependency-free peers subpath with validated metadata and sorted native query schemes in ESM and CommonJS builds.
- Add the headless suite API client, OAuth session handling, state machines, URL guards and en/de/es messages.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/integrations-client@0.10.0
  - @baukit/localization-core@0.10.0
