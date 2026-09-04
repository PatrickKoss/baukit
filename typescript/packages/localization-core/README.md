# `@baukit/localization-core`

`@baukit/localization-core` provides dependency-free locale resolution, catalog key comparison,
stable-code localization, and explicit-locale `Intl` formatter factories. Products supply the
supported locale list, fallback locale, translation catalogs, and copy.

i18next and React composition stays in the consuming app. Resolve and hydrate the locale before
rendering localized UI, then pass that resolved locale to this package's formatter factories.

## Typed catalog segments

`defineCatalogSegment` uses one product-selected reference locale to type every translation. The
supported locale tuple stays in the product. TypeScript rejects missing or extra locale keys,
missing or extra message keys, and a translation that changes a string into a plural message.

```ts
import { defineCatalogSegment } from '@baukit/localization-core';

const accountCatalogs = defineCatalogSegment(
  ['en', 'de'] as const,
  'en',
  { title: 'Account', devices: { one: '1 device', other: '{{count}} devices' } },
  {
    de: { title: 'Konto', devices: { one: '1 Gerät', other: '{{count}} Geräte' } },
  },
);
```

Existing catalog objects can move one segment at a time. Pass the current locale list, reference
locale, reference segment, and translations to `defineCatalogSegment`. Keep translation loading,
i18next resources, namespaces, and copy in the product. The function freezes the returned locale
map but does not deep-freeze or inspect message text.

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

## Boundaries

The package does not include translations, product catalog IDs, persistence, React providers, Expo
Localization, or a localization library. It defines a string plus `{ one, other }` plural shape for
typed catalog segments, but the application still owns plural rules and interpolation syntax.

Every function that depends on a locale or a time zone takes it as an argument. `resolvedTimeZone()`
is the one reader of host state, and it is a separate call you make deliberately. That is what keeps
a test suite from passing in Berlin and failing in CI.
