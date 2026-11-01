---
'@baukit/data-contracts': minor
---

Breaking: the scoped-store registry no longer claims unowned legacy stores. `LegacyStoreOwnership`, `LegacyStoreInspection`, `inspectLegacy` and `legacyStoreName` are gone, and `ResolvedScopedStore` is `{ storeName }` without `claimedLegacy` or `legacyClaimedBySubject`.
