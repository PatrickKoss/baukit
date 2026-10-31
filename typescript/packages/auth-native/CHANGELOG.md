# @baukit/auth-native

## Unreleased

## 0.12.2

### Patch Changes

- Release the coordinated baukit 0.12.2 train.

## 0.12.1

### Patch Changes

- Release the coordinated baukit 0.12.1 train.

## 0.12.0

### Minor Changes

- Release the coordinated baukit 0.12.0 train.

## 0.11.3

### Patch Changes

- Release the coordinated baukit 0.11.3 train.

## 0.11.2

### Patch Changes

- Release the coordinated baukit 0.11.2 train.

## 0.11.1

### Patch Changes

- Release the coordinated baukit 0.11.1 train.

## 0.11.0

### Minor Changes

- Release the coordinated baukit 0.11.0 train.

## 0.10.32

### Patch Changes

- Release the coordinated baukit 0.10.32 train.

## 0.10.31

### Patch Changes

- Release the coordinated baukit 0.10.31 train.

## 0.10.30

### Patch Changes

- Release the coordinated baukit 0.10.30 train.

## 0.10.29

### Patch Changes

- Release the coordinated baukit 0.10.29 train.

## 0.10.28

### Patch Changes

- Release the coordinated baukit 0.10.28 train.

## 0.10.27

### Patch Changes

- Release the coordinated baukit 0.10.27 train.

## 0.10.26

### Patch Changes

- Release the coordinated baukit 0.10.26 train.

## 0.10.25

- Cap the refresh window at half the access token lifetime so short-lived tokens do not refresh on every call.
- Store `issuedAt` in epoch milliseconds and discard persisted sessions without it.

### Patch Changes

- Release the coordinated baukit 0.10.25 train.

## 0.10.24

### Patch Changes

- Release the coordinated baukit 0.10.24 train.

## 0.10.23

### Patch Changes

- Release the coordinated baukit 0.10.23 train.

## 0.10.22

### Patch Changes

- Release the coordinated baukit 0.10.22 train.

## 0.10.21

### Patch Changes

- Release the coordinated baukit 0.10.21 train.

## 0.10.20

### Patch Changes

- Release the coordinated baukit 0.10.20 train.

## 0.10.19

- Add `profile_erased` to session-expired reasons and accept it in `clearSession`. Plain clearing stays silent.
- Persist fresh OIDC authentication after erasure. The next sign-in requests `prompt=login` until a successful sign-in clears the marker.
- Isolate session-expired listener errors in the Clerk adapter. Document Clerk re-onboarding requirements.

### Minor Changes

- 43021fe: Add the `profile_erased` session-expired reason to `clearSession`. OIDC persists a force-login marker and requests `prompt=login` until a successful sign-in. Clerk and browser WorkOS emit the reason; their provider integrations need fresh authentication and JWT `auth_time` for re-onboarding.

### Patch Changes

- Release the coordinated baukit 0.10.19 train.

## 0.10.18

### Patch Changes

- Release the coordinated baukit 0.10.18 train.

## 0.10.17

### Patch Changes

- Release the coordinated baukit 0.10.17 train.

## 0.10.16

### Patch Changes

- Release the coordinated baukit 0.10.16 train.

## 0.10.15

### Patch Changes

- Release the coordinated baukit 0.10.15 train.

## 0.10.14

### Patch Changes

- Release the coordinated baukit 0.10.14 train.

## 0.10.13

### Patch Changes

- Release the coordinated baukit 0.10.13 train.

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
