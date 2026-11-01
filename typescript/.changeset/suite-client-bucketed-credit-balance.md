---
'@baukit/suite-client': minor
---

Breaking: `CreditBalance` mirrors the bucketed balance of Schlüsselbund C21: `suiteSubject`, `wallet`, `revision`, `total`, `grant: { available, expiresAt: number | null }` and `packs: { available, expiresAt }[]`. `available`, `pools`, `CreditPool`, `grant.granted`, `grant.used` and `packs[].remaining` are removed. Read `total` instead of `available` and `packs[].available` instead of `remaining`.
