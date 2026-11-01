# @baukit/purchases-core

## Unreleased

## 0.14.1

### Patch Changes

- Release the coordinated baukit 0.14.1 train.

## 0.14.0

### Minor Changes

- d905e54: Breaking: `VerifiedEntitlement` is `{ mode, suiteSubject, tier, features, revision, plansUrl, billing }`, the body of the kit's `GET /me/entitlement`, and `entitlementFromTicket(claims, plansUrl)` fills it. `CreditBalanceSnapshot` is the bucketed balance of `GET /me/credits`. `PurchaseState` gains `entitlementStatus`, and a failed refresh keeps the last verified entitlement. `PurchasesProvider` requires `productId`, and `useFeature` returns a `FeatureAccess` (`loading`, `unavailable`, `granted`, or `locked` with a paywall) instead of a boolean. New: `LockedFeature`, `featureAccess`, `paywallFor`, which never offers a second purchase for store-managed plans or a purchase under family or organization coverage, `plansLink` for `/plans?from=&feature=`, and `createAccountPlanBackend` with `AccountPlanError`.
- Release the coordinated baukit 0.14.0 train.

## 0.13.0

### Minor Changes

- Release the coordinated baukit 0.13.0 train.

## 0.12.2

### Patch Changes

- Release the coordinated baukit 0.12.2 train.

## 0.12.1

- Add the package.

### Patch Changes

- e4cf9d0: Add `@baukit/purchases-core`, a store-neutral purchase flow gated on the backend-verified entitlement, and `@baukit/purchases-revenuecat`, its `react-native-purchases` adapter.
- Release the coordinated baukit 0.12.1 train.
