# Mobile purchases

**Status:** Adopted.
**Home:** `@baukit/purchases-core` and `@baukit/purchases-revenuecat`.

A product's mobile app sells subscriptions and credit packs through RevenueCat. Schlüsselbund decides what the purchase grants. The app never trusts a RevenueCat entitlement. It waits for the product backend to serve a refreshed entitlement ticket and gates on the feature keys in that ticket.

## Configuration

Generated mobile apps read three variables in `app.config.ts` and pass them as `extra.revenueCat`:

| Variable | Use |
|---|---|
| `EXPO_PUBLIC_REVENUECAT_IOS_API_KEY` | RevenueCat public SDK key for the App Store app |
| `EXPO_PUBLIC_REVENUECAT_ANDROID_API_KEY` | RevenueCat public SDK key for the Play Store app |
| `EXPO_PUBLIC_REVENUECAT_OFFERING_ID` | Optional offering to show. Blank uses RevenueCat's current offering. |

`src/purchases-config.ts` picks the key for `Platform.OS`. A blank key gives the no-op port. Offering and product IDs live in RevenueCat and in the Schlüsselbund catalog (`providerPriceIds.<env>.revenuecat`), never in app code. These are public SDK keys. Keep RevenueCat secret keys and webhook secrets out of the app.

`react-native-purchases` 10.14.0 needs no config plugin. Store purchases need a development or release build, not Expo Go.

## Sign-in and sign-out order

1. Sign the user in and resolve their suite subject, `<identity_domain>|<hub sub>`. This is the ticket `sub`. The hub resolves RevenueCat's `app_user_id`, `original_app_user_id` and aliases against `accounts.suite_subject`, so any other string leaves the purchase unattributed.
2. Call `flow.identify(suiteSubject)`. It runs RevenueCat `logIn`, then reads the entitlement. `purchase`, `restore` and `purchaseCreditPack` reject with `not_identified` until it succeeds. Do not let the user reach the paywall before it resolves.
3. On sign-out, profile erasure or account switch, call `flow.signOut()` before the next user identifies. It clears state and runs `logOut`. The generated `AppShell` calls it when the stored subject becomes null.

An identify for a new user cancels work for the previous one. A late result for the old user rejects with `identity_changed` and never reaches state.

## Paywall data flow

`flow.offerings()` returns `{ current, all }`. Use `selectOffering(offerings, purchasesConfig.offeringId)` to honor the configured offering. Each package carries `priceLabel` from the store, so the app shows store prices and never hard-codes them.

Gate UI with `useFeature('feature.key')` from `@baukit/purchases-core/react`. It reads `features` from the verified entitlement. `useEntitlement()` returns the whole entitlement with `tier` and `revision`.

## Ticket refresh

The product backend owns the hub client. Give the flow an `EntitlementBackend` whose `refreshEntitlement` calls a product route that fetches a fresh ticket from the hub and returns its claims, then pass them through `entitlementFromTicket`. The product backend also updates its entitlement cache from `entitlements.changed` events, as `platform-receiver.md` describes.

After a purchase the flow reads the revision first, buys, then polls `refreshEntitlement` until the revision is newer. The default is 5 reads, 2 seconds apart. The outcome is `granted` with the new entitlement, or `pending` when the webhook has not arrived yet. Show a pending state and let a later customer update or app resume refresh it. RevenueCat customer updates trigger a quiet refresh.

`restore` follows the same path and reports `granted` or `unchanged`.

## Credit packs

Credit packs are RevenueCat non-subscription products. The hub receives them as `NON_RENEWING_PURCHASE` and matches `product_id` to the catalog pack. They change the credit balance, not the entitlement, so `EntitlementBackend.creditBalance` must return `{ suiteSubject, available, revision }` from a product route backed by `credits.changed`. `purchaseCreditPack` accepts only `one_time` packages and reports `credited` once the balance revision advances, or `pending`.

## Errors

Every failure is a `PurchaseError`. Treat `cancelled` as silent. `pending` means the store is waiting for approval, such as Ask to Buy. `network` can be retried. `store` covers other store failures. `unavailable` means the no-op port. `busy` means a purchase is already running.

## No-op mode

Without keys the app uses `createNoopPurchasesPort()`. Identify and sign-out succeed, offerings are empty and purchases reject with `unavailable`. Standalone builds keep working, and `useFeature` reads whatever the backend returns. The generated standalone backend returns no entitlement.
