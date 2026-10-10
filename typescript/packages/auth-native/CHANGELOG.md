# @baukit/auth-native

## Unreleased

## 0.10.12

### Patch Changes

- Release the coordinated baukit 0.10.12 train.

## 0.10.11

### Patch Changes

- Release the coordinated baukit 0.10.11 train.

## 0.10.10

### Patch Changes

- Release the coordinated baukit 0.10.10 train.

## 0.10.9

### Patch Changes

- Release the coordinated baukit 0.10.9 train.

## 0.10.8

### Patch Changes

- Release the coordinated baukit 0.10.8 train.

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

## 0.9.0

- Pass optional API audience and resource parameters through the native browser port and Expo AuthSession.
- Add Clerk hosted Expo sign-in and WorkOS AuthKit public-client PKCE with secure storage, refresh rotation and logout.

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.
- Use SecureStore-safe session and force-login keys. Encode default prefixes and reject invalid custom prefixes at construction. Old keys are not migrated.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- be675db: Raised peer floors to the versions Baukit now tests against. `@baukit/ui-tokens` takes ESLint 10 (`eslint ^10.11.0`), so products can leave ESLint 9. The Expo SDK 57 peers are `react-native ^0.86.3`, `expo-auth-session ^57.0.13`, `expo-secure-store ^57.0.4`, `expo-web-browser ^57.0.3`, `expo-sqlite ^57.0.3`, `expo-notifications ^57.0.21`, and `expo-network ^57.0.2`. `@baukit/data-contracts-dexie` needs `dexie ^4.4.6`.
- Release the coordinated baukit 0.6.0 train.

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
