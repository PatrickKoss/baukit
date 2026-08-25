const CIVIL_DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;
const CIVIL_TIME_PATTERN = /^(?:[01]\d|2[0-3]):[0-5]\d(?::[0-5]\d)?$/;

export const INVALID_CIVIL_DATE_CODE = 'invalid_civil_date';
export type CivilDateValidationCode = typeof INVALID_CIVIL_DATE_CODE;

export type CivilDateParseResult =
  | { readonly ok: true; readonly value: string }
  | { readonly ok: false; readonly code: CivilDateValidationCode };

export function parseCivilDate(civilDate: string): CivilDateParseResult {
  const match = CIVIL_DATE_PATTERN.exec(civilDate);
  if (match === null) {
    return { ok: false, code: INVALID_CIVIL_DATE_CODE };
  }

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const candidate = new Date(Date.UTC(year, month - 1, day));
  const roundTrips =
    candidate.getUTCFullYear() === year &&
    candidate.getUTCMonth() === month - 1 &&
    candidate.getUTCDate() === day;

  return roundTrips ? { ok: true, value: civilDate } : { ok: false, code: INVALID_CIVIL_DATE_CODE };
}

export function isCivilDate(civilDate: string): boolean {
  return parseCivilDate(civilDate).ok;
}

export function civilDateValidationCode(civilDate: string): CivilDateValidationCode | null {
  return parseCivilDate(civilDate).ok ? null : INVALID_CIVIL_DATE_CODE;
}

export function assertCivilDate(civilDate: string): void {
  if (!parseCivilDate(civilDate).ok) {
    throw new RangeError(`Invalid civil date: ${civilDate}`);
  }
}

export function assertCivilTime(civilTime: string | null): void {
  if (civilTime !== null && !CIVIL_TIME_PATTERN.test(civilTime)) {
    throw new RangeError(`Invalid civil time: ${civilTime}`);
  }
}

export function assertTimeZone(timeZone: string): void {
  if (timeZone.trim().length === 0) {
    throw new RangeError('Time zone must be non-empty');
  }

  try {
    new Intl.DateTimeFormat('en-US', { timeZone }).format(new Date(0));
  } catch {
    throw new RangeError(`Invalid IANA time zone: ${timeZone}`);
  }
}

export function addCivilDays(civilDate: string, days: number): string {
  assertCivilDate(civilDate);
  if (!Number.isInteger(days)) {
    throw new RangeError('Days must be an integer');
  }

  const [year, month, day] = civilDate.split('-').map(Number) as [number, number, number];
  const shifted = new Date(Date.UTC(year, month - 1, day + days));
  if (Number.isNaN(shifted.getTime())) {
    throw new RangeError(`Civil date out of range: ${civilDate} + ${String(days)} days`);
  }
  return shifted.toISOString().slice(0, 10);
}

export function civilDateForInstant(instant: Date | string | number, timeZone: string): string {
  assertTimeZone(timeZone);
  const date = instant instanceof Date ? instant : new Date(instant);
  if (Number.isNaN(date.getTime())) {
    throw new RangeError(`Invalid instant: ${String(instant)}`);
  }

  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(date);
  const part = (type: Intl.DateTimeFormatPartTypes): string =>
    parts.find((candidate) => candidate.type === type)?.value ?? '';

  return `${part('year')}-${part('month')}-${part('day')}`;
}

export function civilToday(timeZone: string): string {
  return civilDateForInstant(new Date(), timeZone);
}

export function resolvedTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

export function isInstantOnCivilDate(
  instant: Date | string | number,
  civilDate: string,
  timeZone: string,
): boolean {
  assertCivilDate(civilDate);
  return civilDateForInstant(instant, timeZone) === civilDate;
}

export function compareCivilDates(left: string, right: string): number {
  assertCivilDate(left);
  assertCivilDate(right);
  if (left === right) {
    return 0;
  }
  return left < right ? -1 : 1;
}

export function civilDaysBetween(from: string, to: string): number {
  assertCivilDate(from);
  assertCivilDate(to);
  const millisecondsPerDay = 86_400_000;
  return Math.round(
    (Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / millisecondsPerDay,
  );
}
