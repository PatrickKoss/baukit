---
'@baukit/purchases-core': patch
---

`useOrganizationPlan` returns `OrganizationPlan`, the plan state plus `retry()`, which reads the organization's plan again. Pass the result to `useFeature` or `LockedFeature` as before, and call `retry` from the `unavailable` view (BK30).
