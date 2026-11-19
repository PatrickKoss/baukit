# @baukit/purchases-core

## Unreleased

### Minor Changes

- Add `PurchaseFlow.adoptEntitlement(body)`. A product that already read `GET /me/entitlement` before `identify` hands that body to the flow instead of calling `refreshEntitlement()`, so a cold load makes one request instead of two. The flow parses the body, refuses another subject, keeps the highest revision, and ignores calls after `dispose()`.

## 0.28.3

### Patch Changes

- Release the coordinated baukit 0.28.3 train.
- Updated dependencies
  - @baukit/type-guards@0.28.3

## 0.28.2

### Patch Changes

- Release the coordinated baukit 0.28.2 train.
- Updated dependencies
  - @baukit/type-guards@0.28.2

## 0.28.1

### Patch Changes

- Release the coordinated baukit 0.28.1 train.
- Updated dependencies
  - @baukit/type-guards@0.28.1

## 0.28.0

### Minor Changes

- Release the coordinated baukit 0.28.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.28.0

## 0.27.4

### Patch Changes

- Release the coordinated baukit 0.27.4 train.
- Updated dependencies
  - @baukit/type-guards@0.27.4

## 0.27.3

### Patch Changes

- Release the coordinated baukit 0.27.3 train.
- Updated dependencies
  - @baukit/type-guards@0.27.3

## 0.27.2

### Patch Changes

- Release the coordinated baukit 0.27.2 train.
- Updated dependencies
  - @baukit/type-guards@0.27.2

## 0.27.1

### Patch Changes

- Release the coordinated baukit 0.27.1 train.
- Updated dependencies
  - @baukit/type-guards@0.27.1

## 0.27.0

### Minor Changes

- Release the coordinated baukit 0.27.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.27.0

## 0.26.7

### Patch Changes

- Release the coordinated baukit 0.26.7 train.
- Updated dependencies
  - @baukit/type-guards@0.26.7

## 0.26.6

### Patch Changes

- Release the coordinated baukit 0.26.6 train.
- Updated dependencies
  - @baukit/type-guards@0.26.6

## 0.26.5

### Patch Changes

- Release the coordinated baukit 0.26.5 train.
- Updated dependencies
  - @baukit/type-guards@0.26.5

## 0.26.4

### Patch Changes

- Release the coordinated baukit 0.26.4 train.
- Updated dependencies
  - @baukit/type-guards@0.26.4

## 0.26.3

### Patch Changes

- Release the coordinated baukit 0.26.3 train.
- Updated dependencies
  - @baukit/type-guards@0.26.3

## 0.26.2

### Patch Changes

- Release the coordinated baukit 0.26.2 train.
- Updated dependencies
  - @baukit/type-guards@0.26.2

## 0.26.1

### Patch Changes

- Release the coordinated baukit 0.26.1 train.
- Updated dependencies
  - @baukit/type-guards@0.26.1

## 0.26.0

### Minor Changes

- Release the coordinated baukit 0.26.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.26.0

## 0.25.2

### Patch Changes

- Release the coordinated baukit 0.25.2 train.
- Updated dependencies
  - @baukit/type-guards@0.25.2

## 0.25.1

### Patch Changes

- Release the coordinated baukit 0.25.1 train.
- Updated dependencies
  - @baukit/type-guards@0.25.1

## 0.25.0

### Minor Changes

- Release the coordinated baukit 0.25.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.25.0

## 0.24.0

### Minor Changes

- Release the coordinated baukit 0.24.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.24.0

## 0.23.0

### Minor Changes

- Release the coordinated baukit 0.23.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.23.0

## 0.22.0

### Minor Changes

- Release the coordinated baukit 0.22.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.22.0

## 0.21.0

### Minor Changes

- Release the coordinated baukit 0.21.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.21.0

## 0.20.0

### Minor Changes

- Release the coordinated baukit 0.20.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.20.0

## 0.19.2

### Patch Changes

- Release the coordinated baukit 0.19.2 train.
- Updated dependencies
  - @baukit/type-guards@0.19.2

## 0.19.1

### Patch Changes

- Release the coordinated baukit 0.19.1 train.
- Updated dependencies
  - @baukit/type-guards@0.19.1

## 0.19.0

### Minor Changes

- Release the coordinated baukit 0.19.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.19.0

## 0.18.3

### Patch Changes

- 717f6a1: `useOrganizationPlan` returns `OrganizationPlan`, the plan state plus `retry()`, which reads the organization's plan again. Pass the result to `useFeature` or `LockedFeature` as before, and call `retry` from the `unavailable` view (BK30).
- Release the coordinated baukit 0.18.3 train.

## 0.18.2

### Patch Changes

- Release the coordinated baukit 0.18.2 train.

## 0.18.1

### Patch Changes

- Release the coordinated baukit 0.18.1 train.

## 0.18.0

- Organizations are plan subjects (BK25). `VerifiedEntitlement` gains the optional `organization: {organizationId, name}`. `EntitlementBackend.organizationEntitlement(organizationId)` reads `GET /me/entitlement` with the `X-Organization-Id` header (`ORGANIZATION_HEADER`), and `createAccountPlanBackend` implements it. `paywallFor` answers `covered` with channel `organization` for an organization plan that has the feature and the new `organization_managed` paywall with `organizationId` and `organizationName` when it lacks it, never `purchase`. `organizationFeatureAccess`, `OrganizationPlanState`, `useOrganizationPlan(backend, organizationId)` and the `organization` option of `useFeature` and `LockedFeature` gate on an organization's plan. `ACCOUNT_PLAN_ERROR_CODES` adds `organization_not_accessible` and `organization_id_invalid`. Exhaustive switches over `Paywall['kind']` need the new arm.

### Minor Changes

- Release the coordinated baukit 0.18.0 train.

## 0.17.1

### Patch Changes

- Release the coordinated baukit 0.17.1 train.

## 0.17.0

### Minor Changes

- Release the coordinated baukit 0.17.0 train.

## 0.16.0

### Minor Changes

- Release the coordinated baukit 0.16.0 train.

## 0.15.0

### Minor Changes

- Release the coordinated baukit 0.15.0 train.

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
