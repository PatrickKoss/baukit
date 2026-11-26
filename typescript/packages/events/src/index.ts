import { z } from 'zod';

export const EVENT_SCHEMA_VERSION = 1;
export const MAX_EVENT_ID_CHARACTERS = 64;
export const MAX_EVENT_TYPE_SEGMENT_CHARACTERS = 32;
export const MAX_EVENT_AGE_SECONDS = 7 * 24 * 60 * 60;
export const MAX_EVENT_PAYLOAD_KEYS = 32;
export const MAX_USER_ID_CHARACTERS = 255;
export const MAX_EVENT_CLOCK_SKEW_SECONDS = 5 * 60;

const PAYLOAD_KEY_PATTERN = /^[a-z][a-zA-Z0-9]{0,63}$/u;
const SOURCE_APP_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/u;
const EVENT_TYPE_SEGMENT_PATTERN = /^[a-z][a-z0-9_]*$/u;
const UTC_INSTANT_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:[0-5]\d(?:\.\d{1,3})?Z$/u;
const NEXT_LINE = '\u0085';
const MILLISECONDS_PER_SECOND = 1_000;

export const EVENT_VALIDATION_CODES = [
  'event_id_invalid',
  'event_type_invalid',
  'event_user_mismatch',
  'event_too_old',
  'event_in_future',
  'event_schema_unsupported',
] as const;

export type EventValidationCode = (typeof EVENT_VALIDATION_CODES)[number];
export type EventPayloadValue =
  null | boolean | number | string | EventPayloadValue[] | { [key: string]: EventPayloadValue };

const eventIdSchema = z.string().refine(validEventId, 'event_id_invalid');

const eventTypeSchema = z.string().refine(validEventType, 'event_type_invalid');

export const EventPayloadValueSchema: z.ZodType<EventPayloadValue> = z.lazy(() =>
  z.union([
    z.null(),
    z.boolean(),
    z.number(),
    z.string(),
    z.array(EventPayloadValueSchema),
    z.record(z.string(), EventPayloadValueSchema),
  ]),
);

export const EventPayloadSchema = z
  .record(z.string().refine(matchesWhole(PAYLOAD_KEY_PATTERN)), EventPayloadValueSchema)
  .refine((payload) => Object.keys(payload).length <= MAX_EVENT_PAYLOAD_KEYS, {
    message: `payload must contain at most ${String(MAX_EVENT_PAYLOAD_KEYS)} keys`,
  });

export const EventEnvelopeSchema = z.strictObject({
  eventId: eventIdSchema,
  type: eventTypeSchema,
  userId: z.string().refine(validUserId),
  occurredAt: z.string().refine((value) => parseUtcInstant(value) !== null),
  sourceApp: z.string().refine(matchesWhole(SOURCE_APP_PATTERN)),
  schemaVersion: z
    .number()
    .int()
    .refine((value): boolean => value === EVENT_SCHEMA_VERSION, 'event_schema_unsupported'),
  payload: EventPayloadSchema,
});

export type EventEnvelope = z.infer<typeof EventEnvelopeSchema>;

export const INGEST_OUTCOME_STATUSES = [
  'granted',
  'no_rule',
  'capped',
  'duplicate',
  'rejected',
] as const;

export const IngestOutcomeSchema = z.strictObject({
  outcome: z.enum(INGEST_OUTCOME_STATUSES),
  ledgerEntryId: z.string().nullable(),
});

export type IngestOutcome = z.infer<typeof IngestOutcomeSchema>;

export function validateEventEnvelope(
  envelope: EventEnvelope,
  expectedUserId: string,
  now: string,
): EventValidationCode | null {
  if (envelope.schemaVersion !== EVENT_SCHEMA_VERSION) return 'event_schema_unsupported';
  if (!validEventId(envelope.eventId)) return 'event_id_invalid';
  if (!validEventType(envelope.type)) return 'event_type_invalid';
  if (envelope.userId !== expectedUserId) return 'event_user_mismatch';

  const nowMilliseconds = parseInstant(now);
  const occurredAtMilliseconds = parseInstant(envelope.occurredAt);
  const aheadMilliseconds = occurredAtMilliseconds - nowMilliseconds;
  if (-aheadMilliseconds > MAX_EVENT_AGE_SECONDS * MILLISECONDS_PER_SECOND) return 'event_too_old';
  if (aheadMilliseconds > MAX_EVENT_CLOCK_SKEW_SECONDS * MILLISECONDS_PER_SECOND) {
    return 'event_in_future';
  }
  return null;
}

function matchesWhole(pattern: RegExp): (value: string) => boolean {
  return (value) => pattern.test(value);
}

// Rust's `trim` also strips NEL (U+0085); JavaScript's does not.
function unpadded(value: string): boolean {
  return (
    value.length > 0 &&
    value.trim() === value &&
    !value.startsWith(NEXT_LINE) &&
    !value.endsWith(NEXT_LINE)
  );
}

function validUserId(value: string): boolean {
  return unpadded(value) && Array.from(value).length <= MAX_USER_ID_CHARACTERS;
}

function validEventId(value: string): boolean {
  return unpadded(value) && Array.from(value).length <= MAX_EVENT_ID_CHARACTERS;
}

// Date.parse rolls 2026-02-30 over to March, so the calendar date must round-trip.
function parseUtcInstant(value: string): number | null {
  if (!UTC_INSTANT_PATTERN.test(value)) return null;
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) return null;
  const roundTripped = new Date(milliseconds).toISOString().slice(0, 19);
  return roundTripped === value.slice(0, 19) ? milliseconds : null;
}

function validEventType(value: string): boolean {
  const segments = value.split('.');
  return (
    segments.length === 3 &&
    segments.every(
      (segment) =>
        Array.from(segment).length <= MAX_EVENT_TYPE_SEGMENT_CHARACTERS &&
        matchesWhole(EVENT_TYPE_SEGMENT_PATTERN)(segment),
    )
  );
}

function parseInstant(value: string): number {
  const milliseconds = parseUtcInstant(value);
  if (milliseconds === null) throw new RangeError(`Invalid UTC instant: ${value}`);
  return milliseconds;
}
