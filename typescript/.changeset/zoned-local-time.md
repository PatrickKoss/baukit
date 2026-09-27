---
'@baukit/localization-core': minor
---

Add `resolveZonedLocalTime({ civilDate, civilTime, timeZone, gap, fold })`, which turns a civil date and local time in an IANA zone into an instant. The gap policy (`reject` or `shiftForward`) and the fold policy (`earlier` or `later`) are required and have no default. It returns `{ ok: true, epochMilliseconds, transition }` or `{ ok: false, code }` with `invalid_civil_date`, `invalid_civil_time`, `invalid_time_zone`, or `nonexistent_local_time`, and throws `RangeError` only for an unknown policy value. It reads zone rules through `Intl` and adds no dependency. New exports: `GapPolicy`, `FoldPolicy`, `LocalTimeTransition`, `ZonedLocalTime`, `ZonedLocalTimeCode`, `ZonedLocalTimeResult`, `INVALID_CIVIL_TIME_CODE`, `INVALID_TIME_ZONE_CODE`, and `NONEXISTENT_LOCAL_TIME_CODE`. Shared vectors for this function and for Rust resolvers live in `fixtures/zoned-time/vectors-v1.json`.

No breaking changes. A product with its own `localDateTimeToInstant` can replace it with `resolveZonedLocalTime({ ..., gap: 'reject', fold: 'earlier' })`, which matches the old behavior for valid input, and must then handle the returned error code instead of a thrown `Error`.
