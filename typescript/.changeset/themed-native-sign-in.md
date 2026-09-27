---
'@baukit/auth-native': minor
---

Add a state decoration for themed login pages. `NativeOidcClient.signIn({ stateDecoration })` forwards leading state segments to the browser flow through the new optional `AuthorizationRequest.stateDecoration`. `appearanceStateDecoration({ mode, primaryColor?, secondaryColor? })` builds `ap1.<d|l|s>[.<PRIMARY>.<SECONDARY>]` segments and `decoratedAuthorizationState(segments, entropy)` joins them with a hex nonce of at least 32 random bytes.

`@baukit/auth-native/expo` exports `createExpoBrowserFlow({ randomBytes })`, the AuthSession browser flow that `createExpoOidcEnvironment` already used. With `randomBytes` it puts the decoration in front of a random nonce. Without `randomBytes`, or when the entropy source or a segment fails, it keeps AuthSession's own state. `ExpoOidcEnvironmentOptions` now accepts `randomBytes`.

No breaking changes.
