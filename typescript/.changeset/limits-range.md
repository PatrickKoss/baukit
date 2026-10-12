---
"@baukit/data-contracts": minor
---

Add inclusive `checkRange` checks and range entries in limits policy schemas. Reject inverted
bounds and invalid integers. Map failed range checks to `LimitRangeError` with the `out_of_range`
reason, field, measured value, minimum, and maximum.
