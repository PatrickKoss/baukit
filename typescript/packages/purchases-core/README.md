# `@baukit/purchases-core`

A store-neutral purchase flow for Baukit mobile apps. It defines the `PurchasesPort` that a store adapter implements, a no-op port for builds without store keys, and a `PurchaseFlow` that gates features on the entitlement the product backend verified. It never reads store entitlements. The hub owns entitlements, so a store receipt grants nothing until the refreshed ticket says so.

The package has no Expo, React Native or RevenueCat imports. `@baukit/purchases-revenuecat` is the RevenueCat adapter.

```ts
import {
  PurchaseFlow,
  createNoopPurchasesPort,
  createAccountPlanBackend,
} from '@baukit/purchases-core';

const flow = new PurchaseFlow({
  port: createNoopPurchasesPort(),
  backend: createAccountPlanBackend({
    baseUrl: `${apiOrigin}/api/v1`,
    fetch,
    headers: authHeaders,
  }),
});

await flow.identify(suiteSubject);
const { current } = await flow.offerings();
const outcome = await flow.purchase(current!.packages[0]!);
```

- `identify(suiteSubject)` must finish before `purchase`, `restore` or `purchaseCreditPack`. Without it they reject with `not_identified`.
- `signOut()` clears the state and calls the port's `reset`.
- `purchase` returns `granted` once the backend serves an entitlement with a newer revision, or `pending` after `refreshAttempts` reads (default 5, 2 seconds apart).
- `restore` returns `granted` or `unchanged`.
- `purchaseCreditPack` accepts only `one_time` packages and returns `credited` once the credit balance revision advances, or `pending`.
- Only one purchase runs at a time. A second one rejects with `busy`.
- Failures are `PurchaseError` with `code` `cancelled`, `pending`, `network`, `store`, `unavailable`, `not_identified`, `identity_changed`, `subject_mismatch`, `invalid_subject`, `not_credit_pack` or `busy`.

`createAccountPlanBackend` reads the `GET /me/entitlement` and `GET /me/credits` routes that `baukit-suite`'s `account_plan_router` serves. It treats `suite_identity_missing` as no entitlement and raises `AccountPlanError` with `code` and `retryAfterSeconds` otherwise. A client that verifies tickets itself uses `entitlementFromTicket(claims, plansUrl)`.

`flow.adoptEntitlement(body)` hands the flow an entitlement the product already read from `GET /me/entitlement`, so a cold load does not read it twice. Call it after `identify` resolves, in place of the first `refreshEntitlement()`. It parses `body` with the same checks as `createAccountPlanBackend` and throws `TypeError` for a malformed body, `PurchaseError('not_identified')` before `identify`, and `PurchaseError('subject_mismatch')` for another suite subject. Like a refresh, it keeps the highest revision, so stale data or a slower in-flight refresh never replaces a newer entitlement. After `dispose()` it changes nothing. It returns the entitlement the flow now holds.

`@baukit/purchases-core/react` has `PurchasesProvider` (with `flow` and `productId`), `usePurchaseFlow`, `usePurchaseState`, `useEntitlement`, `useFeature(key)`, `useOrganizationPlan` and `LockedFeature`.

## Configuration in a generated app

Generated mobile apps read three variables in `app.config.ts` and pass them as `extra.revenueCat`:

| Variable | Use |
|---|---|
| `EXPO_PUBLIC_REVENUECAT_IOS_API_KEY` | RevenueCat public SDK key for the App Store app |
| `EXPO_PUBLIC_REVENUECAT_ANDROID_API_KEY` | RevenueCat public SDK key for the Play Store app |
| `EXPO_PUBLIC_REVENUECAT_OFFERING_ID` | Optional offering to show. Blank uses RevenueCat's current offering. |

`src/purchases-config.ts` picks the key for `Platform.OS`. A blank key gives the no-op port. Offering and product IDs live in RevenueCat and in the hub catalog (`providerPriceIds.<env>.revenuecat`), never in app code. These are public SDK keys. Keep RevenueCat secret keys and webhook secrets out of the app. `react-native-purchases` 10.14.0 needs no config plugin. Store purchases need a development or release build, not Expo Go.

## Sign-in and sign-out order

1. Sign the user in and resolve their suite subject, `<identity_domain>|<hub sub>`. This is the ticket `sub`. The hub resolves RevenueCat's `app_user_id`, `original_app_user_id` and aliases against the suite subject, so any other string leaves the purchase unattributed.
2. Call `flow.identify(suiteSubject)`. It runs RevenueCat `logIn`, then reads the entitlement. Do not let the user reach the paywall before it resolves.
3. On sign-out, profile erasure or account switch, call `flow.signOut()` before the next user identifies. The generated `AppShell` calls it when the stored subject becomes null.

An identify for a new user cancels work for the previous one. A late result for the old user rejects with `identity_changed` and never reaches state.

## Gating features

`flow.offerings()` returns `{ current, all }`. `selectOffering(offerings, purchasesConfig.offeringId)` honors the configured offering. Each package carries `priceLabel` from the store, so the app shows store prices and never hard-codes them.

Wrap the app in `<PurchasesProvider flow={flow} productId={productId}>`. Gate UI with `useFeature('feature.key')` or `<LockedFeature feature="feature.key" locked={(paywall) => ...}>`. `useFeature` returns a `FeatureAccess`:

- `loading` while the first refresh for the signed-in subject runs. Show neither the feature nor a paywall.
- `unavailable` when the refresh failed and no verified entitlement exists. Show a retry, never a purchase. This matches the backend's 503 `entitlements_unavailable`.
- `granted` when the verified entitlement lists the key.
- `locked` with a `paywall` otherwise, also when signed out.

A failed refresh keeps the last verified entitlement, as the backend keeps its cached ticket. `LockedFeature` renders its `loading` and `unavailable` props, or nothing.

`paywallFor(entitlement, {productId, feature})` picks the paywall from the `billing` claim:

| Billing | Paywall |
| --- | --- |
| none or `promo` | `purchase` with the Plans link |
| `web` | `manage` with the Plans link, or the claim's `manageUrl` without a plans URL |
| `app_store`, `play_store` | `manage` with `soldBy`. Open the store's subscription settings, never a second purchase. |
| `family`, `organization` | `covered`, no purchase |
| standalone mode | `none` |

An organization's plan ignores `billing`. It answers `covered` with channel `organization` when it has the feature, and `organization_managed` with `organizationId` and `organizationName` when it lacks it. Show that the organization's admins manage the plan. Never offer a purchase.

To gate on an organization the account acts for, load its plan with `useOrganizationPlan(backend, organizationId)`, which calls `backend.organizationEntitlement(organizationId)`. Pass the result as `useFeature(key, { organization })` or `<LockedFeature organization={...}>`. The state is `loading`, `unavailable`, `not_accessible` for a non-member (locked with paywall `none`) or `ready`. The result also has `retry()`, which reads the plan again. Call it from the `unavailable` view. `organizationFeatureAccess(state, key, {productId})` is the same check without React.

`plansLink(plansUrl, {from, feature})` builds `<plansUrl>?from=<product id>&feature=<feature key>` for the hub's Plans page. Both parameters are optional. The builder drops `feature` without `from`, because the hub ignores it there. The hub matches values exactly and shows the normal page for a bad value, so pass the product ID and the catalog feature key unchanged.

## Ticket refresh

The product backend owns the hub client and serves `GET /me/entitlement` and `GET /me/credits` through `account_plan_router`, as Baukit's `docs/platform/platform-receiver.md` describes. It also updates its entitlement cache from `entitlements.changed` events.

After a purchase the flow reads the revision first, buys, then polls `refreshEntitlement` until the revision is newer. A `pending` outcome means the webhook has not arrived yet. Show a pending state and let a later customer update or app resume refresh it. RevenueCat customer updates trigger a quiet refresh.

## Credit packs

Credit packs are RevenueCat non-subscription products. The hub receives them as `NON_RENEWING_PURCHASE` and matches `product_id` to the catalog pack. They change the credit balance, not the entitlement. `EntitlementBackend.creditBalance` returns the bucketed balance `{ suiteSubject, wallet, revision, total, grant, packs }` from `GET /me/credits`, which `credits.changed` keeps current.

## Errors

Treat `cancelled` as silent. `pending` means the store waits for approval, such as Ask to Buy. `network` can be retried. `store` covers other store failures. `unavailable` means the no-op port. `busy` means a purchase is already running.

## No-op mode

Without keys the app uses `createNoopPurchasesPort()`. Identify and sign-out succeed, offerings are empty and purchases reject with `unavailable`. Standalone builds keep working, and `useFeature` reads whatever the backend returns. The generated standalone backend returns no entitlement, so every feature is `locked` with a `none` paywall until the product wires `createAccountPlanBackend`.
