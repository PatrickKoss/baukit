---
'@baukit/suite-client': patch
---

`catalogFeatureKeys`, `featureKeyDrift` and `assertFeatureKeysMatch` count keys from a plan's `organizationProducts` as well as `includedProducts`. A missing or `null` list grants nothing; a list that is not an array fails as `PlanCatalogError`. A product that receives an organization-only key, such as `hebkit.trainer`, must now declare it.
