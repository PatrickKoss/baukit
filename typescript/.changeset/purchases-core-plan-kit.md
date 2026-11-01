---
'@baukit/purchases-core': minor
---

Breaking: `VerifiedEntitlement` is `{ mode, suiteSubject, tier, features, revision, plansUrl, billing }`, the body of the kit's `GET /me/entitlement`, and `entitlementFromTicket(claims, plansUrl)` fills it. `CreditBalanceSnapshot` is the bucketed balance of `GET /me/credits`. `PurchaseState` gains `entitlementStatus`, and a failed refresh keeps the last verified entitlement. `PurchasesProvider` requires `productId`, and `useFeature` returns a `FeatureAccess` (`loading`, `unavailable`, `granted`, or `locked` with a paywall) instead of a boolean. New: `LockedFeature`, `featureAccess`, `paywallFor`, which never offers a second purchase for store-managed plans or a purchase under family or organization coverage, `plansLink` for `/plans?from=&feature=`, and `createAccountPlanBackend` with `AccountPlanError`.
