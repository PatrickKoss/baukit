import { afterEach, describe, expect, it, vi } from 'vitest';

import { AnalyticsClient, followsEventNameConvention } from './client.js';
import { InMemoryAnalyticsStorage } from './storage.js';
import { InMemoryTransport, NoopTransport } from './transports.js';
import type {
  AnalyticsClientOptions,
  AnalyticsContext,
  EventAllowlist,
  Transport,
} from './types.js';

type ProductEvent =
  | {
      name: 'onboarding_started';
      properties: { source: 'organic' | 'invite'; contact?: string; api_token?: string };
    }
  | { name: 'onboarding_completed'; properties: { duration_seconds: number } }
  | { name: 'step_viewed'; properties: { step: number } };

const ANONYMOUS_ID_1 = '00000000-0000-4000-8000-000000000001';
const ANONYMOUS_ID_2 = '00000000-0000-4000-8000-000000000002';
const USER_ID = '10000000-0000-4000-8000-000000000001';

const context: AnalyticsContext = {
  schema_version: 2,
  app: 'example-app',
  app_version: '1.4.0',
  platform: 'web',
  environment: 'test',
  locale: 'en-GB',
};

const allowlist = {
  onboarding_started: ['source', 'contact', 'api_token'],
  onboarding_completed: ['duration_seconds'],
  step_viewed: ['step'],
} as const satisfies EventAllowlist<ProductEvent>;

function createClient(
  overrides: Partial<AnalyticsClientOptions<ProductEvent>> = {},
): AnalyticsClient<ProductEvent> {
  return new AnalyticsClient<ProductEvent>({
    context,
    allowlist,
    uuidFactory: () => ANONYMOUS_ID_1,
    development: false,
    flushBatchSize: 100,
    flushIntervalMs: 60_000,
    ...overrides,
  });
}

afterEach(() => {
  vi.useRealTimers();
});

describe('AnalyticsClient consent', () => {
  it('defaults to unknown and drops rather than buffers without consent', async () => {
    const transport = new InMemoryTransport<ProductEvent>();
    const client = createClient({ transport });

    client.capture({ name: 'onboarding_started', properties: { source: 'organic' } });
    expect(client.consent).toBe('unknown');
    expect(client.pendingCount).toBe(0);

    client.setConsent('granted');
    client.capture({ name: 'onboarding_started', properties: { source: 'invite' } });
    expect(client.pendingCount).toBe(1);
    client.setConsent('denied');
    expect(client.pendingCount).toBe(0);

    client.setConsent('granted');
    client.capture({ name: 'onboarding_completed', properties: { duration_seconds: 12 } });
    await client.flush();

    expect(transport.envelopes).toHaveLength(1);
    expect(transport.envelopes[0]).toMatchObject({
      type: 'capture',
      event: { name: 'onboarding_completed' },
    });
  });

  it('persists and re-evaluates consent and anonymous identity on construction', () => {
    const storage = new InMemoryAnalyticsStorage();
    const first = createClient({ storage });
    first.setConsent('granted');

    const second = createClient({
      storage,
      uuidFactory: () => {
        throw new Error('a stored anonymous UUID should be reused');
      },
    });

    expect(second.consent).toBe('granted');
    expect(second.anonymousId).toBe(first.anonymousId);
  });
});

describe('AnalyticsClient identity', () => {
  it('identifies UUID users, aliases once, and reset rotates the anonymous ID', async () => {
    const transport = new InMemoryTransport<ProductEvent>();
    const uuidValues = [ANONYMOUS_ID_1, ANONYMOUS_ID_2];
    const client = createClient({
      transport,
      uuidFactory: () => uuidValues.shift() ?? ANONYMOUS_ID_2,
      onWarning: (warning) => {
        void warning;
      },
    });
    client.setConsent('granted');
    const originalAnonymousId = client.anonymousId;

    client.identify(USER_ID, { plan: 'free', emailAddress: 'person@example.com' });
    client.alias(originalAnonymousId, USER_ID);
    client.alias(originalAnonymousId, USER_ID);
    client.reset();
    await client.flush();

    expect(client.userId).toBeUndefined();
    expect(client.anonymousId).toBe(ANONYMOUS_ID_2);
    expect(client.anonymousId).not.toBe(originalAnonymousId);
    expect(transport.envelopes.map((envelope) => envelope.type)).toEqual([
      'identify',
      'alias',
      'reset',
    ]);
    expect(transport.envelopes[0]).toMatchObject({
      type: 'identify',
      user_id: USER_ID,
      traits: { plan: 'free', emailAddress: '[redacted]' },
    });
  });

  it('drops identity commands before consent and rejects non-UUID user IDs', async () => {
    const transport = new InMemoryTransport<ProductEvent>();
    const warnings: string[] = [];
    const client = createClient({ transport, onWarning: (warning) => warnings.push(warning) });

    client.identify(USER_ID);
    client.setConsent('granted');
    client.identify('provider-subject-123');
    await client.flush();

    expect(client.userId).toBeUndefined();
    expect(transport.envelopes).toEqual([]);
    expect(warnings).toContain('Dropped analytics identify call because userId is not a UUID.');
  });
});

describe('AnalyticsClient capture pipeline', () => {
  it('drops unknown properties before scrubbing and stamps required context', async () => {
    const transport = new InMemoryTransport<ProductEvent>();
    const client = createClient({ transport });
    client.setConsent('granted');

    client.capture({
      name: 'onboarding_started',
      properties: {
        source: 'organic',
        contact: 'person@example.com',
        api_token: 'short-secret',
        unknown_email: 'leaked@example.com',
      },
    } as ProductEvent);
    await client.flush();

    expect(transport.envelopes[0]).toMatchObject({
      type: 'capture',
      anonymous_id: ANONYMOUS_ID_1,
      event: {
        ...context,
        name: 'onboarding_started',
        properties: {
          source: 'organic',
          contact: '[redacted]',
          api_token: '[redacted]',
        },
      },
    });
    const envelope = transport.envelopes[0];
    expect(envelope?.type === 'capture' && envelope.event.properties).not.toHaveProperty(
      'unknown_email',
    );
  });

  it('drops the oldest waiting command when the bounded queue overflows', async () => {
    const transport = new InMemoryTransport<ProductEvent>();
    const client = createClient({ transport, maxQueueSize: 2, flushBatchSize: 10 });
    client.setConsent('granted');

    client.capture({ name: 'step_viewed', properties: { step: 1 } });
    client.capture({ name: 'step_viewed', properties: { step: 2 } });
    client.capture({ name: 'step_viewed', properties: { step: 3 } });
    expect(client.pendingCount).toBe(2);
    await client.flush();

    expect(
      transport.envelopes.map((envelope) =>
        envelope.type === 'capture' ? envelope.event.properties['step'] : undefined,
      ),
    ).toEqual([2, 3]);
  });

  it('flushes asynchronously at the size threshold and interval', async () => {
    const sizeTransport = new InMemoryTransport<ProductEvent>();
    const sizeClient = createClient({ transport: sizeTransport, flushBatchSize: 2 });
    sizeClient.setConsent('granted');
    sizeClient.capture({ name: 'step_viewed', properties: { step: 1 } });
    sizeClient.capture({ name: 'step_viewed', properties: { step: 2 } });
    await vi.waitFor(() => {
      expect(sizeTransport.envelopes).toHaveLength(2);
    });

    vi.useFakeTimers();
    const intervalTransport = new InMemoryTransport<ProductEvent>();
    const intervalClient = createClient({ transport: intervalTransport, flushIntervalMs: 25 });
    intervalClient.setConsent('granted');
    intervalClient.capture({ name: 'step_viewed', properties: { step: 3 } });
    await vi.advanceTimersByTimeAsync(25);
    expect(intervalTransport.envelopes).toHaveLength(1);
  });
});

describe('AnalyticsClient failure semantics', () => {
  it('caps retries and never exposes transport failures to callers', async () => {
    let calls = 0;
    const failures: number[] = [];
    const failingTransport: Transport<ProductEvent> = {
      send: () => {
        calls += 1;
        throw new Error('provider unavailable');
      },
    };
    const client = createClient({
      transport: failingTransport,
      maxRetries: 2,
      retryDelayMs: 0,
      onTransportFailure: (failure) => {
        failures.push(failure.attempts);
      },
    });
    client.setConsent('granted');

    expect(() => {
      client.capture({ name: 'onboarding_completed', properties: { duration_seconds: 8 } });
    }).not.toThrow();
    await expect(client.flush()).resolves.toBeUndefined();

    expect(calls).toBe(3);
    expect(failures).toEqual([3]);
    expect(client.pendingCount).toBe(0);
  });

  it('provides a no-op transport and advisory event-name convention check', async () => {
    const client = createClient({ transport: new NoopTransport<ProductEvent>() });
    client.setConsent('granted');
    client.capture({ name: 'onboarding_started', properties: { source: 'organic' } });

    await expect(client.flush()).resolves.toBeUndefined();
    expect(followsEventNameConvention('checkout_started')).toBe(true);
    expect(followsEventNameConvention('CheckoutStarted')).toBe(false);
    expect(followsEventNameConvention('checkout_start')).toBe(false);
  });
});
