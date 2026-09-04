# 33. Calendar export

## Question and scope

Should Baukit maintain iCalendar encoders, or can Redemut and Eigenruhe replace their local encoders with maintained libraries? This study compares current Rust and TypeScript libraries against deterministic output, named-time-zone conversion, UTF-8 folding by octet, recurrence, and Baukit's license policy. It covers file export only. Native calendar mutation remains deferred. The experiment details are in the [evidence record](../evidence/33-calendar-export.md).

## Evidence table

| Product or candidate | Files or version | What it proves | Limits or product variation |
| --- | --- | --- | --- |
| Redemut TypeScript | `/home/patrick/projects/redemut/packages/domain/src/calendar-ics.ts` and `test/calendar-ics.test.ts` | Encodes one-off UTC events and weekly `TZID` events, escapes text, folds UTF-8 at 75 octets, injects UID and `DTSTAMP`, rejects a gap, selects the earlier fold instant, and skips a weekly occurrence that falls in a gap. | UID shape, title, description, start selection, gap skipping, and recurrence policy are Redemut choices. |
| Redemut Rust | `/home/patrick/projects/redemut/backend/crates/redemut-domain/src/calendar.rs` | Repeats the TypeScript mechanics with `chrono-tz`, one event per practice weekday, an earlier-fold rule, weekly gap skipping, recurrence, and UTF-8 folding. | The Rust and TypeScript implementations already differ in input shape and event construction. |
| Redemut HTTP feed | `/home/patrick/projects/redemut/backend/crates/redemut-api/src/lib.rs` | Publishes a product calendar feed and owns the token, route, empty-calendar behavior, and plan lookup. | Feed access and product data do not belong in an encoder package. |
| Eigenruhe TypeScript | `/home/patrick/projects/eigenruhe/mobile/src/integrations/ics.ts` and `ics.test.ts` | Encodes one-off and weekly events, creates stable product UIDs, uses named zones, selects the earlier fold instant, rejects gaps, sorts scheduled slots, and folds by UTF-8 octet. | Program and slot mapping, identity hashing, titles, track labels, and event order are product choices. |
| Eigenruhe native adapter | `/home/patrick/projects/eigenruhe/mobile/src/integrations/calendar.ts` | Updates a stored native event, verifies it, deletes a stale event, creates a replacement, records successful items, and falls back to an ICS handoff. | A multi-event run stops on the first failure and returns one aggregate result. Earlier successful items are not reported to the caller as resumable per-item outcomes. Record deletion authority is embedded in the adapter. |
| `icalendar` | Rust `0.17.13`, published 2026-07-28, MIT/Apache-2.0, MSRV 1.88 | Typed events and recurrence, `chrono-tz` values, deterministic property order, CRLF output, and UTF-8-safe octet folding. The pinned experiment passed both DST cases and repeated byte comparison. | An event without explicit UID and timestamp uses a random UUID and current time. Local gap and fold policy must be resolved before constructing `CalendarDateTime`. Event insertion order remains caller-owned. |
| `ics` | Rust `0.5.8`, published 2022-09-25, MIT or Apache-2.0 | Low-level component writer with UTF-8-safe 75-byte folding, `TZID` parameters, `VTIMEZONE`, and raw `RRULE` properties. | It does not resolve named time zones or model recurrence. It moves more RFC assembly into the product and has not released since 2022. |
| `calcard` | Rust `0.3.13`, published 2026-08-25, Apache-2.0 or MIT | Maintained parser, builder, and conversion library for iCalendar, JSCalendar, vCard, and JSContact. | Its conversion scope is much larger than Baukit's export need. The narrower `icalendar` crate passed the required experiment with fewer concepts. |
| `ical-generator` | TypeScript `11.1.1`, published 2026-08-25, MIT | Typed events and weekly recurrence, Temporal-compatible date values, named `TZID` output, UTF-8 octet folding, and deterministic bytes when the caller supplies IDs, stamps, and sorted events. | Construction still calls `crypto.randomUUID()` and `new Date()` before replacing explicit values. Node support is 22 or 24 and later. Expo needs a packaged runtime proof for Web Crypto. Calendar-level timezone configuration can format `DTSTAMP` as local time, so the recipe uses event-level timezones only. |
| `temporal-polyfill` | TypeScript `1.0.4`, published 2026-08-13, MIT | Supplies explicit `reject`, `earlier`, and `later` disambiguation and values accepted by `ical-generator`. | It adds two small runtime dependencies. Native Temporal may replace it only after every supported runtime passes the same vectors. |
| `ics` | TypeScript `3.12.0`, published 2026-04-23, ISC | Maintained event generator with recurrence and caller-supplied UID and timestamp. | It folds by Unicode rune count instead of UTF-8 octets and has no IANA time-zone conversion. A multi-byte content line can exceed 75 octets. |
| `ts-ics` | TypeScript `2.4.6`, published 2026-06-26, MIT | Maintained parser and generator with recurrence and timezone objects. | Its generator counts JavaScript string units when folding. A multi-byte line can exceed 75 octets, so it fails the explicit folding requirement. |
| Tiefgang and Leitbild search | All source under both checked-out products | Searches for `iCalendar`, `.ics`, `VEVENT`, and `RRULE` found no calendar encoder. | Neither product supplies a third event model or adoption target. |

The selected pair is `icalendar` plus `chrono-tz` for Rust and `ical-generator` plus `temporal-polyfill` for TypeScript. All licenses are allowed by `rust/deny.toml`. There is no reason for Baukit to own another RFC 5545 encoder.

## Candidate interface or contract sketch

The useful Baukit deliverable is a calendar export recipe, not public event types. The recipe should pin tested dependency ranges and require these caller rules:

1. The product derives every UID from stable product identity and supplies it explicitly. The encoder never invents identity.
2. The product supplies an explicit `DTSTAMP`, normally the record's stable update timestamp. Tests must not use the clock.
3. The product sorts events by a documented stable key before adding them to the calendar. Library property order is deterministic, but event order follows insertion order.
4. A local civil date and time is resolved before encoding. Gaps return a typed product result such as skip or invalid. Folds require an explicit earlier or later choice. The tested default is earlier.
5. One-off instant events use UTC. Recurring civil-time events use a named `TZID` and a typed recurrence rule. Products decide whether to skip a first occurrence in a gap, move it, or reject the export.
6. Content-line limits are UTF-8 octets, excluding CRLF. A continuation begins with one space or tab and that prefix counts toward the 75-octet physical line.
7. Rust callers set UID and timestamp on every component. TypeScript callers pass `id`, `stamp`, Temporal start and end values, event-level `timezone`, and a sorted event list. They do not set a calendar-level timezone until its `DTSTAMP` behavior is covered by a regression test.
8. Tests compare complete bytes across two fresh encodes and validate every physical line. Tests also inspect semantic fields because the two libraries need not produce identical property order or identical optional RRULE fields.

This contract deliberately does not define a neutral Baukit event. The selected dependencies satisfy the encoder requirements, and their native input types are smaller than an extra cross-runtime model.

## Required-case coverage

| Required case | Evidence and decision |
| --- | --- |
| Deterministic encoding | Both pinned experiments encoded twice in one process and once in two separate processes with identical bytes. Explicit UID, timestamp, and sorted insertion are mandatory because both libraries otherwise create nondeterministic defaults. |
| DST gap | `2026-03-29 02:30 Europe/Berlin` was rejected in Rust and TypeScript. Redemut and Eigenruhe also reject a direct conversion. Whether a weekly schedule skips that occurrence remains product policy. |
| DST fold | `2026-10-25 02:30 Europe/Berlin` resolved to the earlier instant, `2026-10-25T00:30:00Z`, in both experiments. The recipe requires the choice to be explicit. |
| Time-zone conversion | Rust uses `chrono-tz::Tz::from_local_datetime`; TypeScript uses Temporal disambiguation and passes a `ZonedDateTime` to `ical-generator`. A plain JavaScript `Date` plus a timezone string is not accepted as proof of conversion. |
| UTF-8 folding | The title `Grüße 🧘` repeated across the limit produced continuation lines. Rust's longest physical line was 74 octets and TypeScript's was 75. Both stayed within the RFC limit without splitting a UTF-8 scalar. |
| Weekly recurrence | Both outputs contained `RRULE:FREQ=WEEKLY` and a Sunday rule. The libraries differ in optional fields and order, so tests assert meaning rather than cross-language byte parity. |
| One-off event | Each event set included a non-recurring fold event. Product code may encode an instant one-off in UTC even though the experiment retained `TZID` to prove fold selection. |
| UID inputs | Redemut derives practice identifiers, and Eigenruhe hashes domain identity plus provider. These inputs remain local. The shared rule is only that the UID is stable and explicitly passed. |
| Event ordering | Redemut emits practice-day input order. Eigenruhe sorts scheduled slots but accepts caller order in the generic encoder. The recipe requires a product-documented stable sort before library calls. |
| License | The Rust experiment passed `cargo deny` with Baukit's license configuration. MIT, Apache-2.0, and ISC are allowed. A product lockfile still needs its normal dependency audit. |
| Supported runtime | `icalendar` MSRV 1.88 is below Baukit's Rust 1.95 floor. `ical-generator` supports Node 22 and 24 or later. Browser bundling and Expo Web Crypto must pass packed-artifact tests before adoption claims those runtimes. |
| Native per-item outcomes | Eigenruhe can persist several successful items and then fall back after a later failure, but exposes only one aggregate outcome. It may delete a stale stored event before replacement. No native adapter extraction is approved until results name each attempted item and an ownership contract bounds update and delete operations. |

Calendar-client interoperability also needs an adoption test in the calendar applications each product supports. The tested output uses `TZID` without embedding `VTIMEZONE`, matching the current product files. If a supported client requires embedded transitions, that is a separate compatibility requirement and should use a maintained timezone component generator.

## Decision

Decision: contract or recipe. Publish a platform recipe for `icalendar` with `chrono-tz` and `ical-generator` with `temporal-polyfill`, including the deterministic caller rules and the two DST vectors. Do not add a Baukit encoder or shared event model. Before TypeScript adoption, run a packed `ical-generator` smoke test in the generated web build and Expo Android and iOS builds, with `crypto.randomUUID` available. Defer native calendar adapters until they return resumable per-item results and state exactly which owned records they may update or delete.

## What stays product-owned

- Stable UID inputs and namespaces, `PRODID`, titles, descriptions, locations, routes, file names, feed tokens, and calendar selection.
- Product plans, practice days, slots, recurrence end rules, gap behavior, fold choice when earlier is not suitable, duration, all-day behavior, and event sort key.
- Export eligibility, sharing or download UX, access control, persistence records, analytics, retry timing, and calendar-client support.
- Native permission prompts, provider choice, matching, ownership markers, update and delete authority, partial-failure recovery, and fallback to file export.
