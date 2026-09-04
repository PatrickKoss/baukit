# 33. Calendar export

## Source product files

- `/home/patrick/projects/redemut/packages/domain/src/calendar-ics.ts`
- `/home/patrick/projects/redemut/packages/domain/test/calendar-ics.test.ts`
- `/home/patrick/projects/redemut/backend/crates/redemut-domain/src/calendar.rs`
- `/home/patrick/projects/redemut/backend/crates/redemut-api/src/lib.rs`
- `/home/patrick/projects/redemut/docs/baukit-playback-audit.md`
- `/home/patrick/projects/eigenruhe/mobile/src/integrations/ics.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/integrations/ics.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/integrations/calendar.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/integrations/calendar.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/integrations/sharing.ts`
- `/home/patrick/projects/eigenruhe/docs/BAUKIT_FEEDBACK_AUDIT.md`

Searches across Tiefgang and Leitbild found no iCalendar encoder. Search terms were `iCalendar`, `.ics`, `VEVENT`, and `RRULE`.

## Observed failure or repeated glue

Redemut maintains separate Rust and TypeScript encoders for escaping, UTC and named-zone formatting, folding, recurrence, UID handling, and DST resolution. Eigenruhe maintains a third TypeScript encoder with the same mechanics. The product policies differ. Eigenruhe's native multi-event export records successful items but reports one aggregate result and may delete a stale stored event before creating its replacement.

## Baukit owner

The owner should be a platform recipe, not a runtime crate or package. It should name the tested dependency pair and deterministic-use rules. No Baukit owner is accepted for native calendar mutation.

## Public types and errors

No new public Baukit types or errors are proposed. Products use the libraries' event and recurrence types, then map validation and DST failures to product errors. A future native adapter would need an owned-record identity, a per-item `created`, `updated`, `unchanged`, `removed`, `denied`, or `failed` outcome, and bounded safe error codes before this decision is revisited.

## Product-owned inputs

Products supply stable UID inputs, product ID, event sort key, title, description, location, routes, duration, calendar name and selection, one-off or recurrence choice, recurrence end, gap and fold policy, file name, feed authentication, sharing behavior, and native record ownership.

## Concurrency, failure, privacy, and cleanup cases

The recipe needs deterministic repeated-run tests, duplicate and reordered events, invalid zones and dates, DST gaps and folds, one-off and recurring events, multi-byte folding, empty calendars, invalid recurrence, and missing explicit UID or timestamp. Errors and logs must omit event text, routes, tokens, and user identity. File encoding has no cleanup beyond caller-owned temporary files. Native cleanup stays deferred because partial success, retry, stale identifiers, permission loss, update verification, and deletion authority are not yet represented as per-item outcomes.

## Supported runtimes

Rust: `icalendar 0.17.13` has MSRV 1.88 and fits Baukit's Rust 1.95 floor. TypeScript: `ical-generator 11.1.1` declares Node 22 or 24 and later. Its module uses global `crypto.randomUUID()` while constructing every event, even with an explicit ID. Browser and React Native support therefore require packed web and Expo device tests. `temporal-polyfill 1.0.4` supplies DST disambiguation until every supported runtime provides compatible Temporal behavior.

## Product adoption change

Redemut can delete `packages/domain/src/calendar-ics.ts` after callers construct `ical-generator` events directly; its tests become product mapping and compatibility tests. In `backend/crates/redemut-domain/src/calendar.rs`, the escaping, formatting, folding, and component assembly can be deleted while the practice-plan mapping stays. Eigenruhe can delete the generic escaping, formatting, folding, and component assembly from `mobile/src/integrations/ics.ts`; program mapping, stable product identity, sort policy, sharing, and native adapter code remain. No native calendar file becomes deletable under this decision.

## Throwaway experiments

All files were created under `/tmp/baukit-calendar-study`. Registry metadata was read on 2026-09-04. Candidate releases were:

| Package | Version | Published | License | Result |
| --- | ---: | --- | --- | --- |
| Rust `icalendar` | 0.17.13 | 2026-07-28 | MIT/Apache-2.0 | Selected. |
| Rust `ics` | 0.5.8 | 2022-09-25 | MIT or Apache-2.0 | Rejected for no named-zone resolution or typed recurrence. |
| Rust `calcard` | 0.3.13 | 2026-08-25 | Apache-2.0 or MIT | Maintained but broader than the selected builder. |
| TypeScript `ical-generator` | 11.1.1 | 2026-08-25 | MIT | Selected, with runtime proof still required for Expo. |
| TypeScript `temporal-polyfill` | 1.0.4 | 2026-08-13 | MIT | Selected for explicit gap and fold resolution. |
| TypeScript `ics` | 3.12.0 | 2026-04-23 | ISC | Rejected because folding counts runes, not octets, and it lacks IANA conversion. |
| TypeScript `ts-ics` | 2.4.6 | 2026-06-26 | MIT | Rejected because folding counts JavaScript string units, not octets. |

The Rust experiment ran with `cargo 1.97.1` and `rustc 1.97.1`. Its manifest pinned these dependencies:

```toml
[dependencies]
chrono = "=0.4.45"
chrono-tz = "=0.10.4"
icalendar = { version = "=0.17.13", default-features = false, features = ["recurrence", "chrono-tz"] }
```

Commands:

```sh
cargo build --manifest-path /tmp/baukit-calendar-study/rust/Cargo.toml
cargo run --manifest-path /tmp/baukit-calendar-study/rust/Cargo.toml --locked --quiet > /tmp/baukit-calendar-study/rust-run-1.txt
cargo run --manifest-path /tmp/baukit-calendar-study/rust/Cargo.toml --locked --quiet > /tmp/baukit-calendar-study/rust-run-2.txt
sha256sum /tmp/baukit-calendar-study/rust-run-1.txt /tmp/baukit-calendar-study/rust-run-2.txt
cmp -s /tmp/baukit-calendar-study/rust-run-1.txt /tmp/baukit-calendar-study/rust-run-2.txt && echo rust_two_process_runs_identical=true
cargo deny --manifest-path /tmp/baukit-calendar-study/rust/Cargo.toml --config /home/patrick/projects/baukit/rust/deny.toml check advisories licenses
```

Result:

```text
gap=nonexistent
fold_earlier=2026-10-25T00:30:00+00:00
deterministic=true
bytes=713
max_physical_octets=74
continuation_lines=2
has_weekly_rrule=true
a1819ce6a0de1a91b1628e4491e83e50d54eb978925083f7d32cd1c0eb410667  /tmp/baukit-calendar-study/rust-run-1.txt
a1819ce6a0de1a91b1628e4491e83e50d54eb978925083f7d32cd1c0eb410667  /tmp/baukit-calendar-study/rust-run-2.txt
rust_two_process_runs_identical=true
advisories ok, licenses ok
```

The TypeScript setup pinned these dependencies and ran on Node `v24.20.0`:

```sh
cd /tmp/baukit-calendar-study/typescript
npm init -y
npm install --save-exact ical-generator@11.1.1 temporal-polyfill@1.0.4
node index.mjs > /tmp/baukit-calendar-study/ts-run-1.txt
node index.mjs > /tmp/baukit-calendar-study/ts-run-2.txt
sha256sum /tmp/baukit-calendar-study/ts-run-1.txt /tmp/baukit-calendar-study/ts-run-2.txt
cmp -s /tmp/baukit-calendar-study/ts-run-1.txt /tmp/baukit-calendar-study/ts-run-2.txt && echo ts_two_process_runs_identical=true
npm ls --all --omit=dev
```

Result:

```text
added 4 packages, audited 5 packages, 0 vulnerabilities
gap=nonexistent
fold_earlier=2026-10-25T00:30:00Z
deterministic=true
bytes=724
max_physical_octets=75
continuation_lines=2
has_weekly_rrule=true
sha256=959fbe8cf957d3ae06edf68f0009cddc1467817836ec8fd01ff1b08197ae7ab5
955dbe366799bb671799d65ab38f7e629e487bd058e5da3d77603e8e609dd772  /tmp/baukit-calendar-study/ts-run-1.txt
955dbe366799bb671799d65ab38f7e629e487bd058e5da3d77603e8e609dd772  /tmp/baukit-calendar-study/ts-run-2.txt
ts_two_process_runs_identical=true
ical-generator@11.1.1
temporal-polyfill@1.0.4
  temporal-spec@1.0.1
  temporal-utils@1.0.2
```

The event set contained a weekly Sunday event beginning at `2026-04-05 02:30 Europe/Berlin`, after rejecting the spring gap, and a one-off event at the earlier instance of the autumn fold. The weekly title repeated `Grüße 🧘` across the fold boundary. Both programs supplied fixed UIDs and a `2026-03-20T12:00:00Z` timestamp and sorted events before insertion.
