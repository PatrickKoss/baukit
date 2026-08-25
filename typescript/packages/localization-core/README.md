# @baukit/localization-core

`@baukit/localization-core` provides dependency-free locale resolution, catalog key comparison,
stable-code localization, and explicit-locale `Intl` formatter factories. Products supply the
supported locale list, fallback locale, translation catalogs, and copy.

The package does not include translations, product catalog IDs, persistence, React providers,
Expo Localization, or a localization library. It also does not choose plural or unit policy for
an application.

i18next and React composition stays in the consuming app. Resolve and hydrate the locale before
rendering localized UI, then pass that resolved locale to this package's formatter factories.

## Civil dates

A civil date is a `YYYY-MM-DD` calendar day with no time and no offset. Diary entries, plan days,
and reminder dates are civil dates, and computing one with `Date.getDate()` or an ISO string slice
gets the wrong answer for any user whose zone disagrees with the host.

```ts
import { addCivilDays, civilDateForInstant, civilToday } from '@baukit/localization-core';

civilToday('Pacific/Auckland'); // '2026-08-25'
civilDateForInstant('2026-08-25T06:00:00Z', 'America/Los_Angeles'); // '2026-08-24'
addCivilDays('2026-03-07', 1); // '2026-03-08', DST does not shift a calendar day
```

- `parseCivilDate(date)` returns `{ ok: true, value }` or `{ ok: false, code: 'invalid_civil_date' }`.
  `isCivilDate`, `civilDateValidationCode`, and `assertCivilDate` are the boolean, code, and
  throwing forms. All of them reject impossible days such as `2026-02-30`.
- `assertCivilTime(time | null)` accepts `HH:MM` and `HH:MM:SS`.
- `addCivilDays(date, days)`, `civilDaysBetween(from, to)`, and `compareCivilDates(left, right)`
  work on the calendar, so a DST transition never adds or drops a day.
- `civilDateForInstant(instant, timeZone)` accepts a `Date`, an ISO string, or epoch
  milliseconds. `civilToday(timeZone)` is the same call against now, and
  `isInstantOnCivilDate(instant, date, timeZone)` answers whether a timestamp belongs to a local
  day.
- `assertTimeZone(zone)` validates an IANA zone. `resolvedTimeZone()` reads the host zone; every
  other function takes the zone explicitly, so nothing silently depends on where the process runs.
