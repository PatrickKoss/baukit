# `@baukit/purchases-core`

A store-neutral purchase flow for Baukit mobile apps. It defines the `PurchasesPort` that a store adapter implements, a no-op port for builds without store keys, and a `PurchaseFlow` that gates features on the entitlement the product backend verified. It never reads store entitlements. Schlüsselbund owns entitlements, so a store receipt grants nothing until the refreshed ticket says so.

The package has no Expo, React Native or RevenueCat imports. `@baukit/purchases-revenuecat` is the RevenueCat adapter.

```ts
import {
  PurchaseFlow,
  createNoopPurchasesPort,
  entitlementFromTicket,
} from '@baukit/purchases-core';

const flow = new PurchaseFlow({
  port: createNoopPurchasesPort(),
  backend: {
    refreshEntitlement: async () => entitlementFromTicket(await api.refreshEntitlementTicket()),
    creditBalance: () => api.creditBalance(),
  },
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

`@baukit/purchases-core/react` has `PurchasesProvider`, `usePurchaseFlow`, `usePurchaseState`, `useEntitlement` and `useFeature(key)`.

Read Baukit's `docs/platform/mobile-purchases.md` for the full wiring.
