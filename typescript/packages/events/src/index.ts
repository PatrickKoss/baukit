import { z } from 'zod';

export const EVENT_SCHEMA_VERSION = 1;
export const MAX_EVENT_ID_CHARACTERS = 64;
export const MAX_EVENT_TYPE_SEGMENT_CHARACTERS = 32;
export const MAX_EVENT_AGE_SECONDS = 7 * 24 * 60 * 60;
export const MAX_EVENT_PAYLOAD_KEYS = 32;

export const EVENT_VALIDATION_CODES = [
  'event_id_invalid',
  'event_type_invalid',
  'event_user_mismatch',
  'event_too_old',
  'event_schema_unsupported',
] as const;

export type EventValidationCode = (typeof EVENT_VALIDATION_CODES)[number];
export type EventPayloadValue =
  null | boolean | number | string | EventPayloadValue[] | { [key: string]: EventPayloadValue };

const eventIdSchema = z.string().superRefine((value, context) => {
  if (!validEventId(value)) {
    context.addIssue({ code: z.ZodIssueCode.custom, message: 'event_id_invalid' });
  }
});

const eventTypeSchema = z.string().superRefine((value, context) => {
  if (!validEventType(value)) {
    context.addIssue({ code: z.ZodIssueCode.custom, message: 'event_type_invalid' });
  }
});

export const EventPayloadValueSchema: z.ZodType<EventPayloadValue> = z.lazy(() =>
  z.union([
    z.null(),
    z.boolean(),
    z.number(),
    z.string(),
    z.array(EventPayloadValueSchema),
    z.record(EventPayloadValueSchema),
  ]),
);

export const EventPayloadSchema = z
  .record(z.string().regex(/^[a-z][a-z0-9_]{0,63}$/u), EventPayloadValueSchema)
  .refine((payload) => Object.keys(payload).length <= MAX_EVENT_PAYLOAD_KEYS, {
    message: `payload must contain at most ${String(MAX_EVENT_PAYLOAD_KEYS)} keys`,
  });

export const EventEnvelopeSchema = z
  .object({
    event_id: eventIdSchema,
    type: eventTypeSchema,
    user_id: z.string().trim().min(1).max(255),
    occurred_at: z.string().datetime(),
    source_app: z.string().regex(/^[a-z][a-z0-9_-]{0,31}$/u),
    schema_version: z
      .number()
      .int()
      .superRefine((value, context) => {
        if (value !== EVENT_SCHEMA_VERSION) {
          context.addIssue({
            code: z.ZodIssueCode.custom,
            message: 'event_schema_unsupported',
          });
        }
      }),
    payload: EventPayloadSchema,
  })
  .strict();

export type EventEnvelope = z.infer<typeof EventEnvelopeSchema>;

export const INGEST_OUTCOME_STATUSES = [
  'granted',
  'no_rule',
  'capped',
  'duplicate',
  'rejected',
] as const;

export const IngestOutcomeSchema = z
  .object({
    outcome: z.enum(INGEST_OUTCOME_STATUSES),
    ledger_entry_id: z.string().nullable(),
  })
  .strict();

export type IngestOutcome = z.infer<typeof IngestOutcomeSchema>;

export function validateEventEnvelope(
  envelope: EventEnvelope,
  expectedUserId: string,
  now: string,
): EventValidationCode | null {
  if (envelope.schema_version !== EVENT_SCHEMA_VERSION) return 'event_schema_unsupported';
  if (!validEventId(envelope.event_id)) return 'event_id_invalid';
  if (!validEventType(envelope.type)) return 'event_type_invalid';
  if (envelope.user_id !== expectedUserId) return 'event_user_mismatch';

  const nowMilliseconds = parseInstant(now);
  const occurredAtMilliseconds = parseInstant(envelope.occurred_at);
  if (nowMilliseconds - occurredAtMilliseconds > MAX_EVENT_AGE_SECONDS * 1_000) {
    return 'event_too_old';
  }
  return null;
}

function validEventId(value: string): boolean {
  return (
    value.length > 0 &&
    value.trim() === value &&
    Array.from(value).length <= MAX_EVENT_ID_CHARACTERS
  );
}

function validEventType(value: string): boolean {
  const segments = value.split('.');
  return (
    segments.length === 3 &&
    segments.every(
      (segment) =>
        Array.from(segment).length <= MAX_EVENT_TYPE_SEGMENT_CHARACTERS &&
        /^[a-z][a-z0-9_]*$/u.test(segment),
    )
  );
}

function parseInstant(value: string): number {
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) throw new RangeError(`Invalid RFC 3339 instant: ${value}`);
  return milliseconds;
}
