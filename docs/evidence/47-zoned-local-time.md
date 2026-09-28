# Zoned local time evidence

Item 11 of the [cross-product feature plan](../cross-product-feature-plan.md).

## Source revisions

All cited files were clean in their working trees.

- Eigenruhe `f74cebb`: `mobile/src/integrations/ics.ts:49-235` (`localDateTimeToInstant` at
  `:148-195`, ICS helpers at `:49-146` and `:197-235`), callers in
  `mobile/src/integrations/calendar.ts:90,121`, tests in `mobile/src/integrations/ics.test.ts`.
  Backend: `backend/crates/eigenruhe-services/src/notifications.rs:131-175,681-824`.
- Hebkit `841bf5d`: `mobile/src/integrations/ics.ts:27-186` (`localDateTimeToInstant` at
  `:154-186`, ICS helpers at `:27-146` and `:188` onward), callers in
  `mobile/src/features/reminders/scheduler.ts:136,202,257`, `mobile/src/integrations/calendar.ts:216`,
  `mobile/src/integrations/files/ics.ts:288,341,455,866`, and
  `mobile/src/integrations/files/gpx.ts:279`. Backend:
  `backend/crates/hebkit-postgres/src/adapters/postgres/nutrition.rs:1114-1117`.
- Redemut `a782538`: `packages/domain/src/calendar-ics.ts:110-167` (`localDateTimeToInstant` and
  `nextWeeklyOccurrence`), caller in `mobile/src/calendar-export.ts:195`. Backend:
  `backend/crates/redemut-domain/src/calendar.rs:122-128`.
- Leitbild `bd38b33`: `backend/crates/leitbild-services/src/reminder.rs:197-254` and the test at
  `:370-386`.
- Also read for the Rust survey: Tiefgang `backend/crates/tiefgang-api/src/stats_models.rs:72-79`
  and `backend/crates/tiefgang-postgres/src/credits.rs:169-172`; Solo Leveling System
  `backend/crates/sl-services/src/quests.rs:387-390`.
- [Study 33](../studies/33-calendar-export.md) and [its evidence](33-calendar-export.md).
- Runtime used for the runs below: Node 24.20.0, ICU 78.3, tzdata 2026c, `temporal-polyfill` 1.0.4,
  `ical-generator` 11.1.1, `chrono` 0.4 with `chrono-tz` 0.10.4.

## Shared vectors

`fixtures/zoned-time/vectors-v1.json` holds 38 cases. Each case gives a civil date, a local time, a
zone, the transition it hits (`none`, `gap`, `fold`, or `null` for invalid input), and the expected
result for all four gap and fold policy pairs, either an RFC 3339 UTC instant or an error code.

| Group | Cases |
|---|---|
| Spring-forward gaps | Berlin 02:30, gap start 02:00, last second 02:59:59, gap end 03:00; New York 02:30; Santiago midnight gap 00:30 |
| Fall-back folds | Berlin 02:30, fold start 02:00, 02:30:15, fold end 03:00; New York 01:30; Santiago 23:30 |
| Half-hour and 45-minute offsets | Kolkata and Kathmandu (no transition); Adelaide gap and fold; Lord Howe 30-minute gap and fold; Chatham gap and fold |
| Rule changes | Apia skipped 2011-12-30 entirely (a 24-hour gap); Moscow's 2014 permanent move from +04 to +03 (a fold without DST); Sao Paulo midnight gap in 2018 and the same date in 2019 after Brazil dropped DST |
| Invalid zone | unknown `Mars/Olympus_Mons`, empty string, offset string `+01:00` |
| Invalid date or time | `2026-02-30`, `2026-6-15`, `24:00`, `23:59:60`, `9:00`, `09:00:00.500`, `09:00+01:00` |
| Control | `UTC` |

`temporal-polyfill` generated the expected instants (`later` disambiguation for a shifted gap,
`earlier` and `later` for folds). The generator script stays outside the repository, because
`temporal-polyfill` is not a Baukit dependency. A `chrono-tz` 0.10.4 probe classified every valid
case the same way and returned the same earliest and latest instants, so the file is usable from
Rust as it stands. `chrono` has no shift-forward mode, so the probe did not check those four-case
columns. The new Baukit function matches every outcome in the file without using Temporal.

## Today's TypeScript differences

A throwaway script copied the three `localDateTimeToInstant` functions unchanged and ran every case.

| Input class | Eigenruhe | Hebkit | Redemut |
|---|---|---|---|
| Unique times, all zones | correct | correct | correct |
| Gaps (all 10 gap cases) | throws `Error` "does not exist in this timezone" | throws `Error` "does not exist in the selected timezone" | throws `Error` "does not exist in the selected timezone" |
| Folds (all 9 fold cases) | earlier instant | earlier instant | earlier instant |
| Unknown or empty zone | raw `RangeError` from `Intl` | raw `RangeError` from `Intl` | `Error` "does not exist", same as a gap |
| Offset string `+01:00` | accepted, returns an instant | accepted | accepted |
| `2026-02-30`, `24:00` | `Error` "does not exist", same as a gap | same | same |
| `23:59:60` | `Error` "does not exist" | same | returns `21:59:59Z`, silently one second early |
| `2026-6-15`, `9:00`, fractions, offsets | shape error | shape error | shape error |

All three copies already implement the same policy for valid input: reject gaps, take the earlier
fold. They disagree only on invalid input. The plan read this as "Redemut rejects nonexistent
times", but all three do. The real defects:

- No copy tells a caller which policy it applied. Nothing in the signature says gaps throw and folds
  pick the earlier instant.
- An impossible date, hour 24, and an unknown zone (Redemut) produce the same message as a DST gap.
  Redemut's `nextWeeklyOccurrence` matches on the substring `does not exist`, so an unknown zone makes
  it skip four weeks and then fail with "Could not find a valid weekly calendar occurrence".
- Redemut silently turns a leap second into `:59`, because `Temporal.PlainDateTime.from` constrains
  second 60.
- All three accept an offset string as a zone on Node 24. Engines that predate offset-zone support
  in ECMA-402 reject it, so the same input can pass in tests and fail on a device.

## Rust behaviors

| Product | Code | Gap | Fold |
|---|---|---|---|
| Leitbild | `reminder.rs:236-254` `resolve_local` | steps forward one minute at a time, up to 180 minutes | earlier |
| Redemut | `calendar.rs:122-128` | on `LocalResult::None`, tries the same weekday a week later | earlier |
| Hebkit | `nutrition.rs:1114-1117` | `.single()` fails | `.single()` fails |
| Tiefgang | `stats_models.rs:72-79`, `credits.rs:169-172` | `.earliest()` returns `None`, mapped to an error | earlier |
| Solo Leveling System | `quests.rs:387-390` | `None` | earlier |
| Eigenruhe | `notifications.rs` | no local-to-instant resolution | no local-to-instant resolution |

The plan cites Eigenruhe `notifications.rs:681-824` for `.single()`. Those lines are test setup on
`Utc.with_ymd_and_hms(...)`, which never has a gap or fold. The service itself only converts instants
to local time (`now.with_timezone(&timezone)` at `:138`), which is always unambiguous. Every other
`.single()` in the Eigenruhe backend is also on `Utc`. Eigenruhe's Rust code has no DST resolution
to fix.

Leitbild's stepping differs from the RFC 5545 reading on every gap case except the one that starts
exactly at the gap start. The probe ran its algorithm with `chrono-tz`:

- Berlin 02:30 becomes 03:00 local (`01:00Z`). The vectors say `01:30Z`, which is 03:30 local.
- Berlin 02:59:59 becomes `01:00:59Z`, because stepping keeps the seconds and lands 59 seconds past
  the first valid minute.
- Adelaide, Lord Howe, Chatham, Santiago, and Sao Paulo each land at the first valid minute after the
  gap, not one gap length later.
- Apia's 24-hour gap is longer than the 180-minute cap, so the function returns `InvalidSchedule`.

`chrono` parsing also differs from the vectors. `NaiveDate::parse_from_str("2026-6-15", "%Y-%m-%d")`,
`NaiveTime::parse_from_str("9:00", "%H:%M")`, and `"23:59:60"` all succeed (the last as a leap
second). A Rust test against the vectors must parse strictly before calling `from_local_datetime`.

## Rust helper deferred

No Rust helper lands in this item. The plan's rule is that a Rust helper waits until two Rust
products agree on the policy, and the item's evidence names two Rust products (Leitbild and
Eigenruhe) that do not: Leitbild steps through gaps, and Eigenruhe has no resolver at all.

The wider survey above shows that Redemut, Tiefgang, and Solo Leveling System already resolve with
the same core semantics, reject the gap and take the earlier fold. Redemut then tries the next week
instead of failing. That is exactly `chrono`'s `from_local_datetime(..).earliest()`, so a Baukit helper for that
policy would wrap one existing call and add little. The useful shared piece for Rust is the vector
file plus strict parsing. Whether a helper is still worth adding for `shiftForward`, or for the
parse-and-resolve step, is an open decision for the plan owner. Until then, Rust resolvers test
against `fixtures/zoned-time/vectors-v1.json`.

## Baukit owner

`@baukit/localization-core`, new module `typescript/packages/localization-core/src/zoned-time.ts`,
next to `civil-date.ts`.

## Policy decision

Two gap policies and two fold policies, both required, as the plan's contract specifies.

- `shiftForward` reads the local time with the UTC offset in effect before the gap. Berlin 02:30
  becomes 03:30, not 03:00. This matches RFC 5545 section 3.3.5 and Temporal's `compatible` and
  `later` disambiguation for gaps, so an instant from this function agrees with what a calendar
  client shows for the same `TZID` time. Leitbild's "first valid minute" is a third
  policy that no other product uses and that no calendar client produces. It is left out.
- `reject` returns `nonexistent_local_time`. It does not throw, so a planner can skip or report the
  occurrence without string matching.
- `earlier` and `later` pick the first or second instant of a fold. The RFC 5545 reading of a `TZID`
  time is `earlier`.
- A policy value outside the two unions throws `RangeError`. TypeScript already forbids it, and
  treating it as data would hide a caller bug behind a result the caller may not check.

## Time-zone data source decision

Use `Intl.DateTimeFormat` with `timeZone` and `formatToParts`, and add no dependency.

- `Intl` is enough for the algorithm. The function needs only the UTC offset at a given instant. It
  reads the wall clock at three instants (the target read as UTC, and one day either side), derives
  up to three offsets, and keeps each candidate instant whose wall clock round-trips to the target.
  Zero candidates is a gap, two is a fold. The shift-forward instant uses the offset one day before.
  The Temporal specification uses the same one-day-either-side offsets to size a gap.
- `temporal-polyfill` would not change the data source. It reads offsets from
  `Intl.DateTimeFormat.prototype.formatToParts` too (`global.js` in 1.0.4), so it would add two
  runtime dependencies to a dependency-free package and still read the same tz database.
- Native `Temporal` is absent from Node 24 (`typeof Temporal === 'undefined'` on 24.20.0) and from
  Hermes, so it cannot be the baseline.
- Browsers: current Chrome, Firefox, and Safari ship `Intl.DateTimeFormat` with IANA zones and
  `formatToParts`.
- Node: official Node 24 builds include full ICU. The test suite runs every vector on Node 24.
- Expo and Hermes: Hermes implements `Intl.DateTimeFormat` with `timeZone` and `formatToParts` on
  Android through the platform ICU and on iOS through Foundation. `civilDateForInstant` in this
  package already depends on the same calls. Eigenruhe's and Hebkit's shipped Expo builds run the
  same `Intl` offset search in `localDateTimeToInstant` today.
- The formatter pins `calendar: 'gregory'`, `numberingSystem: 'latn'`, and `hourCycle: 'h23'`, and
  treats an hour of `24` as `0`, so a device locale with another calendar or digits cannot change the
  parsed parts.
- Offset strings are rejected before `Intl` sees them. ECMA-402 added offset zones such as `+01:00`
  in 2024, so engines differ on whether they accept one, and `chrono-tz` rejects it. Rejecting it
  everywhere keeps the answer independent of the engine.

Residual risk: this change did not run the vectors inside a Hermes build, because no Hermes VM is
available on this workstation and an emulator run is outside this item's gates. Item 12 ships an
Expo package, and the delivery rules put Expo packages under native Android and iOS checks. Running
`fixtures/zoned-time/vectors-v1.json` in those checks closes the gap. Each runtime also reads its own tz database: the device OS on mobile, ICU in
Node and browsers, and a compiled table in `chrono-tz`. The vectors use settled historical
transitions and 2026 rules. A future rule change could make runtimes disagree until each updates.

## Public types and errors

- `resolveZonedLocalTime(input: ZonedLocalTime): ZonedLocalTimeResult`.
- `ZonedLocalTime`: `{ civilDate, civilTime, timeZone, gap: GapPolicy, fold: FoldPolicy }`.
- `GapPolicy = 'reject' | 'shiftForward'`, `FoldPolicy = 'earlier' | 'later'`,
  `LocalTimeTransition = 'none' | 'gap' | 'fold'`.
- `ZonedLocalTimeResult`: `{ ok: true, epochMilliseconds, transition }` or `{ ok: false, code }`.
- `ZonedLocalTimeCode`: `invalid_civil_date` (the existing `INVALID_CIVIL_DATE_CODE`),
  `invalid_civil_time`, `invalid_time_zone`, `nonexistent_local_time`, exported as
  `INVALID_CIVIL_TIME_CODE`, `INVALID_TIME_ZONE_CODE`, and `NONEXISTENT_LOCAL_TIME_CODE`.

The result carries epoch milliseconds, not a `Date`, so it is immutable and compares with `===`.
Item 12 compares instants by value when it computes keep, cancel, and schedule sets.

## Supported runtimes

Browsers, Node 24, and Expo with Hermes, the runtimes `@baukit/localization-core` already supports.
The function is pure: it reads no clock, no host zone, and no global state, and it creates one
formatter per call.

## Failure behavior

Every data problem returns a typed code, checked in this order: civil date, civil time, zone. A gap
under `reject` returns `nonexistent_local_time`. Only an unknown policy value throws. Civil dates
follow `parseCivilDate`, which accepts years 0100 to 9999.

## Privacy boundary

The function takes a date, a time, and a zone name and returns a number or a code. It logs nothing
and returns no input text in its errors.

## Breaks

None. The function and its types are additive. The changeset records the adoption change products
make when they replace their own copies.

## Calendar export recipe

Study 33's accepted recipe is now [`docs/platform/calendar-export-recipe.md`](../platform/calendar-export-recipe.md),
linked from the root README's documentation index and the localization contract. Changes from the
study text:

- Local times resolve through `resolveZonedLocalTime` in TypeScript. The Rust section names the
  `LocalResult` handling, strict parsing, and the shared vectors.
- It states that `{ gap: 'shiftForward', fold: 'earlier' }` matches how a calendar client reads a
  `TZID` time, and that a `later` fold choice cannot be expressed as `TZID`.
- A smoke run of the recipe with `ical-generator` 11.1.1 encoded identical bytes twice, and its
  longest physical line was 74 octets. The same run showed a pitfall. A recurring event that starts
  at Berlin 02:30 on the fold day and lasts 30 minutes encodes as `DTSTART;TZID=Europe/Berlin:20261025T023000`
  and `DTEND;TZID=Europe/Berlin:20261025T020000`. A client reads that end as the first 02:00, before
  the start. The recipe tells products not to let a recurring `TZID` event span a fold.
- It says hand-written ICS escaping, folding, UTC formatting, and UID helpers should move to the
  recommended library, without naming products.

## Product adoption change

Deferred to the products, as step 4 of the item:

- Eigenruhe: replace `localDateTimeToInstant` and its `partsAt` and `sameParts` helpers
  (`mobile/src/integrations/ics.ts:104-195`) with `resolveZonedLocalTime`, choosing policies at
  `calendar.ts:90,121`. Replace the hand-written encoder (`ics.ts:49-102,138-142,197-235`) with
  `ical-generator` and `temporal-polyfill` per the recipe. Keep `calendar.ts` local.
- Hebkit: replace `localDateTimeToInstant` (`mobile/src/integrations/ics.ts:124-186`) at every
  caller. The reminder scheduler (`scheduler.ts:136,202,257`) is notification planning and should
  adopt together with item 12. Replace the export helpers (`stableCalendarUid`, `escapeIcsText`,
  `foldIcsLine`, `formatIcsUtc`, `formatIcsLocal`, `encodeIcsCalendar`) with `ical-generator`.
  `unescapeIcsText` and `unfoldIcsLines` serve the ICS importer in `files/ics.ts`, which the
  recommended library does not cover, so they stay until an import recipe exists. Keep
  `calendar.ts` local.
- Redemut: replace `localDateTimeToInstant` (`packages/domain/src/calendar-ics.ts:110-132`) with
  `resolveZonedLocalTime({ gap: 'reject', fold: 'earlier' })` and change `nextWeeklyOccurrence` to
  skip only on `nonexistent_local_time`, not on a message substring. Redemut already uses the
  recommended encoder.
- Leitbild: no TypeScript change. Its Rust `resolve_local` can test against the vectors; moving from
  minute stepping to the RFC 5545 reading is a product decision recorded here, not a Baukit change.

The plan's acceptance requires the three TypeScript copies to be replaced in at least two products.
That happens in the product adoption step, not in this change.

## Follow-up (2026-09-28)

The residual risk above is closed for Android. `fixtures/zoned-time/vectors-v1.json` now runs inside
Hermes on an Android emulator, and Hermes agrees with Node on every vector: 152 of 152 checks (38
cases times four policy pairs) passed, with the device zone at `Europe/Berlin`. No package change
was needed, because no vector disagreed.

What changed and why:

- The vector runner moved out of `zoned-time.test.ts` into `src/zoned-time-vectors.ts`, exported as
  `@baukit/localization-core/vectors` with `zonedTimeVectorChecks(fixture)` and the fixture types.
  Each check has a `label`, an `expected` value, and an `actual()` call. The Vitest suite now runs
  these checks with `toEqual`, and the device app runs the same checks with a canonical JSON
  comparison, so Node and Hermes share one set of assertions instead of two copies.
- `examples/expo-notifications-conformance` runs the checks in a debug build, where the engine is
  Hermes; the app fails if `HermesInternal` is missing. Expected instants come from `Date.parse` in
  the same runtime, so the app also fails if any expected instant parses to `NaN`, which would
  otherwise compare equal on both sides.
- `make expo-notifications-conformance` runs it, and CI has a matching job. Item 12's follow-up in
  `48-local-notifications.md` explains why this is a second app rather than part of the SQLite
  conformance app.

Gates: `make expo-notifications-conformance` passed (vectors marker
`{"zonedTime":152,"notificationPlan":88,"deviceZone":"Europe/Berlin"}` in both launches), the whole
TypeScript workspace `check` (build, lint, test, format:check) passed, and localization-core runs
265 tests.

Breaks: none. The root export is unchanged; `./vectors` is a new subpath, recorded in
`typescript/.changeset/vector-check-exports.md`.

Still open: this was an Android emulator (API 36 image `baukit-api-36`), not a physical device, and
it read the emulator image's ICU time-zone data. iOS was not run, since it needs macOS with Xcode,
so Hermes on iOS, which reads zones through Foundation, is still unchecked.
