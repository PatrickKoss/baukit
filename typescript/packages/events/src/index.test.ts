import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  EVENT_SCHEMA_VERSION,
  EventEnvelopeSchema,
  MAX_EVENT_AGE_SECONDS,
  MAX_EVENT_ID_CHARACTERS,
  MAX_EVENT_PAYLOAD_KEYS,
  MAX_EVENT_TYPE_SEGMENT_CHARACTERS,
  validateEventEnvelope,
  type EventEnvelope,
  type EventValidationCode,
} from './index.js';

interface FixtureDocument {
  contract_version: number;
  representation: Record<string, unknown>;
  constants: {
    schema_version: number;
    maximum_event_id_characters: number;
    maximum_event_type_segment_characters: number;
    maximum_event_age_seconds: number;
    maximum_payload_keys: number;
  };
  cases: {
    name: string;
    input: { envelope: EventEnvelope; expected_user_id: string; now: string };
    expected_code: EventValidationCode | null;
  }[];
}

const fixtureUrl = new URL('../../../../fixtures/events/event-envelope-v1.json', import.meta.url);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixtures = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as FixtureDocument;

describe('event envelope fixtures', () => {
  it('pins the contract representation and constants', () => {
    expect(fixtures.contract_version).toBe(EVENT_SCHEMA_VERSION);
    expect(fixtures.representation['timestamps']).toBe('rfc3339_utc_z');
    expect(fixtures.constants).toEqual({
      schema_version: EVENT_SCHEMA_VERSION,
      maximum_event_id_characters: MAX_EVENT_ID_CHARACTERS,
      maximum_event_type_segment_characters: MAX_EVENT_TYPE_SEGMENT_CHARACTERS,
      maximum_event_age_seconds: MAX_EVENT_AGE_SECONDS,
      maximum_payload_keys: MAX_EVENT_PAYLOAD_KEYS,
    });
  });

  it.each(fixtures.cases)('$name', ({ input, expected_code: expectedCode }) => {
    expect(validateEventEnvelope(input.envelope, input.expected_user_id, input.now)).toBe(
      expectedCode,
    );
  });

  it.each(fixtures.cases.filter(({ expected_code: code }) => code === null))(
    'zod accepts $name',
    ({ input }) => {
      expect(EventEnvelopeSchema.parse(input.envelope)).toEqual(input.envelope);
    },
  );

  it.each(
    fixtures.cases.filter(
      ({ expected_code: code }) =>
        code === 'event_id_invalid' ||
        code === 'event_type_invalid' ||
        code === 'event_schema_unsupported',
    ),
  )('zod reports the stable code for $name', ({ input, expected_code: expectedCode }) => {
    const result = EventEnvelopeSchema.safeParse(input.envelope);
    expect(result.success).toBe(false);
    if (result.success) return;
    expect(result.error.issues.map(({ message }) => message)).toContain(expectedCode);
  });
});
