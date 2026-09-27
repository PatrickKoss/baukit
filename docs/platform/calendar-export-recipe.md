# Calendar export recipe

**Status:** Recipe. Baukit ships no iCalendar encoder and no shared event model.
**Applies to:** products that export `.ics` files or feeds from Rust or TypeScript.
**Related:** [localization contract](./localization-contract.md), shared vectors in
`fixtures/zoned-time/vectors-v1.json`.

Two maintained libraries already produce correct RFC 5545 output when the caller supplies stable
inputs. Use them. Products that grew their own text escaping, line folding, UTC formatting, and UID
hashing should delete those helpers and move to the library for their runtime.

| Runtime | Encoder | Zone data | Tested version |
|---|---|---|---|
| Rust | `icalendar` with `default-features = false` and the `recurrence` and `chrono-tz` features | `chrono-tz` | `icalendar` 0.17.13, `chrono-tz` 0.10.4 |
| TypeScript | `ical-generator` | `temporal-polyfill` for `ZonedDateTime` values, `@baukit/localization-core` for resolution | `ical-generator` 11.1.1, `temporal-polyfill` 1.0.4 |

All four are licensed MIT or Apache-2.0, and the Rust pair passes `rust/deny.toml`. The Rust `ics`
crate moves too much RFC assembly into the product and has not released since 2022. The TypeScript
`ics` and `ts-ics` packages fold lines by characters instead of UTF-8 octets, so a line with
multi-byte text can exceed 75 octets.

## Caller rules

1. Derive every UID from stable product identity and pass it explicitly. Both libraries invent a
   random UID when you leave it out.
2. Pass an explicit `DTSTAMP`, normally the record's stable update time. Tests never read the
   clock.
3. Sort events by a documented stable key before adding them. Property order inside an event is
   deterministic in both libraries; event order is insertion order.
4. Resolve a civil date and local time to an instant before encoding, under an explicit gap and
   fold policy. See [Resolving local times](#resolving-local-times).
5. Encode a one-off event as a UTC instant. Encode a recurring civil-time event with a named
   `TZID` and a typed recurrence rule. The product decides whether a first occurrence that falls in
   a gap is skipped, moved, or rejected.
6. Content lines are limited to 75 UTF-8 octets, excluding CRLF. A continuation line starts with one
   space or tab, and that prefix counts toward the limit.
7. In Rust, set UID and timestamp on every component. In TypeScript, pass `id`, `stamp`, Temporal
   start and end values, event-level `timezone`, and a sorted event list. Do not set a
   calendar-level `timezone` in `ical-generator`: it can format `DTSTAMP` as local time.
8. Tests encode twice and compare complete bytes, check every physical line length in octets, and
   assert semantic fields. Rust and TypeScript output need not match byte for byte, because the
   libraries order properties and optional `RRULE` parts differently.

## Resolving local times

In TypeScript, call `resolveZonedLocalTime` from `@baukit/localization-core`. It takes the civil
date, the local time, the IANA zone, a gap policy (`reject` or `shiftForward`), and a fold policy
(`earlier` or `later`). Neither policy has a default. The result is
`{ ok: true, epochMilliseconds, transition }` or `{ ok: false, code }`, so a gap under `reject`
returns `nonexistent_local_time` instead of throwing. Do not resolve with a `Date` built from a
string, a fixed offset, or a hand-rolled offset search.

In Rust, resolve with `chrono_tz::Tz::from_local_datetime` and match all three `LocalResult`
variants. `.single()` turns every fold into an error, and stepping minute by minute through a gap
lands on the first valid minute, not on the RFC 5545 instant. Parse dates and times strictly first:
`chrono` format strings accept `2026-6-15`, `9:00`, and a leap second `23:59:60`, all of which the
shared vectors treat as invalid. Test the resolver against `fixtures/zoned-time/vectors-v1.json`.
Baukit adds a Rust helper once two Rust products agree on one policy.

A calendar client reads a `TZID` time by RFC 5545 section 3.3.5 rules: a time in a gap uses the
offset from before the gap, and a time in a fold means the first occurrence. That is
`{ gap: 'shiftForward', fold: 'earlier' }`. Use that pair when the instant you store or notify on
must match what the calendar shows for a recurring `TZID` event. A `later` fold choice cannot be
expressed as a `TZID` time. Encode that occurrence in UTC, or pick a different time.

An event that starts inside a fold and ends after it can show an end time before its start time in
local wall-clock terms, for example `DTSTART;TZID=Europe/Berlin:20261025T023000` with
`DTEND;TZID=Europe/Berlin:20261025T020000`. Clients read the end as the first 02:00, which is before
the start. Do not let a recurring `TZID` event span a fold. Move it, or encode that occurrence in
UTC.

```ts
import ical, { ICalCalendarMethod, ICalEventRepeatingFreq, ICalWeekday } from 'ical-generator';
import { Temporal } from 'temporal-polyfill';
import { resolveZonedLocalTime } from '@baukit/localization-core';

function encodeWeeklySession(session: Session, timeZone: string): string | null {
  const resolved = resolveZonedLocalTime({
    civilDate: session.firstDate,
    civilTime: session.localTime,
    timeZone,
    gap: 'shiftForward',
    fold: 'earlier',
  });
  if (!resolved.ok) {
    return null;
  }

  const start = Temporal.Instant.fromEpochMilliseconds(
    resolved.epochMilliseconds,
  ).toZonedDateTimeISO(timeZone);
  const calendar = ical({ method: ICalCalendarMethod.PUBLISH, prodId: '//Product//Export//EN' });
  calendar.createEvent({
    id: session.uid,
    stamp: Temporal.Instant.from(session.updatedAt),
    start,
    end: start.add({ minutes: session.durationMinutes }),
    timezone: timeZone,
    summary: session.title,
    repeating: { freq: ICalEventRepeatingFreq.WEEKLY, byDay: [ICalWeekday.SU] },
  });
  return calendar.toString();
}
```

## Supported runtimes

`icalendar` supports Rust 1.88 and later, below Baukit's 1.95 floor. `ical-generator` supports Node
22 and 24 and later. Its constructor still calls `crypto.randomUUID()` and `new Date()` before your
explicit values replace them, so a web or Expo product must prove the packed library in its own web
build and in Android and iOS builds with `crypto.randomUUID` available before it claims those
runtimes.

The tested output uses `TZID` without an embedded `VTIMEZONE`. If a calendar client the product
supports needs embedded transitions, add a maintained time-zone component generator for that
client instead of writing transitions by hand.

## Native calendar adapters

Adapters that write to the device calendar stay in the product. Baukit will not extract one until it
reports a result for each attempted item, so a partial failure can resume, and states exactly which
owned events it may update or delete.

## What stays in the product

- UID inputs and namespaces, `PRODID`, titles, descriptions, locations, routes, file names, feed
  tokens, and calendar selection.
- Plans, slots, recurrence end rules, gap and fold choices, duration, all-day behavior, and the event
  sort key.
- Export eligibility, share or download flow, access control, persistence, analytics, and retry
  timing.
- Native permission prompts, provider choice, event matching, ownership markers, update and delete
  authority, and fallback to file export.
