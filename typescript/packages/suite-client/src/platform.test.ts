import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import {
  PLATFORM_CREDIT_ERROR_CODES,
  PLATFORM_EVENT_TYPES,
  type CreditBalance,
  type CreditReservation,
  type OrganizationSummary,
  type PlatformEventPayloads,
} from './index.js';

function fixture(path: string): unknown {
  return JSON.parse(readFileSync(new URL(`../../../../fixtures/${path}`, import.meta.url), 'utf8'));
}
interface Catalog {
  types: { type: string; fields: { name: string }[] }[];
}
interface Envelope {
  type: string;
  payload: Record<string, unknown>;
}
interface Recorded {
  responses: Record<string, { status: number; body: unknown }>;
}
const catalog = fixture('platform-events/v1/catalog.json') as Catalog;
const envelopes = fixture('platform-events/v1/envelopes.json') as Envelope[];
const recorded = fixture('platform-api/v1/responses.json') as Recorded;
const suffix = (type: string): string => type.slice(type.indexOf('.') + 1);
const body = (name: string): unknown => recorded.responses[name]?.body;

it('lists every platform catalog event type', () => {
  expect(catalog.types.map((entry) => suffix(entry.type))).toEqual(PLATFORM_EVENT_TYPES);
});

it('types the new event payloads with the catalog fields', () => {
  const organization: PlatformEventPayloads['organization.updated'] = {
    organizationId: '018f1234-5678-7abc-8def-012345678901',
    revision: 7,
  };
  const credits: PlatformEventPayloads['credits.changed'] = {
    suiteSubject: 'suite|person-123',
    wallet: 'hebkit',
    revision: 4,
  };
  for (const [type, payload] of [
    ['organization.updated', organization],
    ['credits.changed', credits],
  ] as const) {
    const entry = catalog.types.find((candidate) => suffix(candidate.type) === type);
    expect(Object.keys(payload).sort()).toEqual(entry?.fields.map((field) => field.name).sort());
    expect(envelopes.find((envelope) => suffix(envelope.type) === type)?.payload).toEqual(payload);
  }
});

it('matches recorded hub credit and organization responses', () => {
  const balance: CreditBalance = {
    suiteSubject: 'suite-dev|alice',
    wallet: 'hebkit',
    available: 10,
    revision: 0,
    pools: [],
    grant: { granted: 0, used: 0, available: 0 },
    packs: [{ remaining: 10, expiresAt: 1795392000 }],
  };
  const pending: CreditReservation = {
    id: '018f1234-5678-7abc-8def-012345678901',
    wallet: 'hebkit',
    amount: 4,
    charged: null,
    status: 'pending',
    expiresAt: 1792800900,
  };
  const summary: OrganizationSummary = {
    organizationId: '018f1234-5678-7abc-8def-012345678901',
    status: 'active',
    productId: 'runtime_analyzer',
    tier: 'professional',
    features: [],
    paidQuantity: 4,
    assignedSeats: 1,
    revision: 1,
  };
  expect(body('creditBalance')).toEqual(balance);
  expect(body('reservationPending')).toEqual(pending);
  expect(body('reservationSettled')).toEqual({ ...pending, charged: 4, status: 'settled' });
  expect(body('organizationSummary')).toEqual(summary);
});

it('lists the recorded credit error codes', () => {
  const codes = Object.values(recorded.responses)
    .map((entry) => (entry.body as { error?: { code: string } }).error?.code)
    .filter((code): code is string => code !== undefined);
  for (const code of PLATFORM_CREDIT_ERROR_CODES) expect(codes).toContain(code);
  expect([...PLATFORM_CREDIT_ERROR_CODES].sort()).toEqual(PLATFORM_CREDIT_ERROR_CODES);
});
