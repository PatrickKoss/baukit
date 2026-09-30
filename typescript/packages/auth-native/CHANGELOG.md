# @baukit/auth-native

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.
- 118e055: Add a state decoration for themed login pages. `NativeOidcClient.signIn({ stateDecoration })` forwards leading state segments to the browser flow through the new optional `AuthorizationRequest.stateDecoration`. `appearanceStateDecoration({ mode, primaryColor?, secondaryColor? })` builds `ap1.<d|l|s>[.<PRIMARY>.<SECONDARY>]` segments and `decoratedAuthorizationState(segments, entropy)` joins them with a hex nonce of at least 32 random bytes.

  `@baukit/auth-native/expo` exports `createExpoBrowserFlow({ randomBytes })`, the AuthSession browser flow that `createExpoOidcEnvironment` already used. With `randomBytes` it puts the decoration in front of a random nonce. Without `randomBytes`, or when the entropy source or a segment fails, it keeps AuthSession's own state. `ExpoOidcEnvironmentOptions` now accepts `randomBytes`.

  No breaking changes.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/auth-native`.
