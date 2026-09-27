---
'@baukit/data-contracts': minor
---

Add `parseLimitsPolicy`, `LimitsPolicyError`, `LimitError`, and `enforceLimit` to `/limits`. `parseLimitsPolicy(value, schema)` validates a limits policy file against a schema that names the version, the keys of each section, and the keys allowed to be zero. It returns a result typed from the schema. `enforceLimit(field, reason, check)` turns `LimitExceededError` into `LimitError` with `reason`, `field`, `measured`, and `allowed`.

The web and mobile templates now call these instead of generating their own parser. Products that copied the template's `parseLimitsPolicy`, `LimitsPolicyError`, and `LimitError` can delete them and declare a schema. `LimitError` gains `measured` and `allowed`, and invalid counts now throw the package's `RangeError` message (`measured must be a non-negative safe integer`) instead of the generated `<field> count must be a non-negative integer`.

No breaking changes to existing package exports.
