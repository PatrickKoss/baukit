import { INVALID_CIVIL_DATE_CODE, parseCivilDate } from './civil-date.js';

export type GapPolicy = 'reject' | 'shiftForward';
export type FoldPolicy = 'earlier' | 'later';
export type LocalTimeTransition = 'none' | 'gap' | 'fold';

export const INVALID_CIVIL_TIME_CODE = 'invalid_civil_time';
export const INVALID_TIME_ZONE_CODE = 'invalid_time_zone';
export const NONEXISTENT_LOCAL_TIME_CODE = 'nonexistent_local_time';

export type ZonedLocalTimeCode =
  | typeof INVALID_CIVIL_DATE_CODE
  | typeof INVALID_CIVIL_TIME_CODE
  | typeof INVALID_TIME_ZONE_CODE
  | typeof NONEXISTENT_LOCAL_TIME_CODE;

export interface ZonedLocalTime {
  readonly civilDate: string;
  readonly civilTime: string;
  readonly timeZone: string;
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
}

export type ZonedLocalTimeResult =
  | {
      readonly ok: true;
      readonly epochMilliseconds: number;
      readonly transition: LocalTimeTransition;
    }
  | { readonly ok: false; readonly code: ZonedLocalTimeCode };

interface WallClock {
  readonly year: number;
  readonly month: number;
  readonly day: number;
  readonly hour: number;
  readonly minute: number;
  readonly second: number;
}

const CIVIL_TIME_PATTERN = /^([01]\d|2[0-3]):([0-5]\d)(?::([0-5]\d))?$/;
const OFFSET_TIME_ZONE_PATTERN = /^[+-]/;
const GAP_POLICIES: readonly string[] = ['reject', 'shiftForward'] satisfies GapPolicy[];
const FOLD_POLICIES: readonly string[] = ['earlier', 'later'] satisfies FoldPolicy[];
const HOURS_PER_DAY = 24;
const MILLISECONDS_PER_DAY = 86_400_000;
const OFFSET_SAMPLE_DISTANCES = [-MILLISECONDS_PER_DAY, 0, MILLISECONDS_PER_DAY];

export function resolveZonedLocalTime(input: ZonedLocalTime): ZonedLocalTimeResult {
  assertPolicies(input.gap, input.fold);
  const date = parseCivilDate(input.civilDate);
  if (!date.ok) {
    return { ok: false, code: INVALID_CIVIL_DATE_CODE };
  }
  const time = CIVIL_TIME_PATTERN.exec(input.civilTime);
  if (time === null) {
    return { ok: false, code: INVALID_CIVIL_TIME_CODE };
  }
  const formatter = zoneFormatter(input.timeZone);
  if (formatter === null) {
    return { ok: false, code: INVALID_TIME_ZONE_CODE };
  }

  const [year, month, day] = input.civilDate.split('-').map(Number) as [number, number, number];
  const wall = utcMilliseconds({
    year,
    month,
    day,
    hour: Number(time[1]),
    minute: Number(time[2]),
    second: Number(time[3] ?? 0),
  });
  return resolveWall(formatter, wall, input);
}

function assertPolicies(gap: unknown, fold: unknown): void {
  if (typeof gap !== 'string' || !GAP_POLICIES.includes(gap)) {
    throw new RangeError(`Unknown gap policy: ${String(gap)}`);
  }
  if (typeof fold !== 'string' || !FOLD_POLICIES.includes(fold)) {
    throw new RangeError(`Unknown fold policy: ${String(fold)}`);
  }
}

function resolveWall(
  formatter: Intl.DateTimeFormat,
  wall: number,
  policy: Pick<ZonedLocalTime, 'gap' | 'fold'>,
): ZonedLocalTimeResult {
  const candidates = candidateInstants(formatter, wall);
  const earliest = candidates[0];
  const latest = candidates[candidates.length - 1];
  if (earliest === undefined || latest === undefined) {
    return resolveGap(formatter, wall, policy.gap);
  }
  if (earliest === latest) {
    return { ok: true, epochMilliseconds: earliest, transition: 'none' };
  }
  const epochMilliseconds = policy.fold === 'earlier' ? earliest : latest;
  return { ok: true, epochMilliseconds, transition: 'fold' };
}

function resolveGap(
  formatter: Intl.DateTimeFormat,
  wall: number,
  gap: GapPolicy,
): ZonedLocalTimeResult {
  if (gap === 'reject') {
    return { ok: false, code: NONEXISTENT_LOCAL_TIME_CODE };
  }
  const offsetBeforeGap = offsetAt(formatter, wall - MILLISECONDS_PER_DAY);
  return { ok: true, epochMilliseconds: wall - offsetBeforeGap, transition: 'gap' };
}

function candidateInstants(formatter: Intl.DateTimeFormat, wall: number): number[] {
  const offsets = new Set(
    OFFSET_SAMPLE_DISTANCES.map((distance) => offsetAt(formatter, wall + distance)),
  );
  return [...offsets]
    .map((offset) => wall - offset)
    .filter((instant) => localWallAt(formatter, instant) === wall)
    .sort((left, right) => left - right);
}

function offsetAt(formatter: Intl.DateTimeFormat, instant: number): number {
  return localWallAt(formatter, instant) - instant;
}

function localWallAt(formatter: Intl.DateTimeFormat, instant: number): number {
  const parts = formatter.formatToParts(new Date(instant));
  const part = (type: Intl.DateTimeFormatPartTypes): number =>
    Number(parts.find((candidate) => candidate.type === type)?.value);
  return utcMilliseconds({
    year: part('year'),
    month: part('month'),
    day: part('day'),
    hour: part('hour') % HOURS_PER_DAY,
    minute: part('minute'),
    second: part('second'),
  });
}

function utcMilliseconds(wall: WallClock): number {
  const date = new Date(0);
  date.setUTCFullYear(wall.year, wall.month - 1, wall.day);
  date.setUTCHours(wall.hour, wall.minute, wall.second, 0);
  return date.getTime();
}

function zoneFormatter(timeZone: string): Intl.DateTimeFormat | null {
  if (OFFSET_TIME_ZONE_PATTERN.test(timeZone)) {
    return null;
  }
  try {
    return new Intl.DateTimeFormat('en-US', {
      timeZone,
      calendar: 'gregory',
      numberingSystem: 'latn',
      hourCycle: 'h23',
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  } catch {
    return null;
  }
}
