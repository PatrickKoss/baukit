import { AnalyticsClient } from '@baukit/analytics-core';
import type { AnalyticsContext, AnalyticsEnvelope, EventAllowlist } from '@baukit/analytics-core';
import { describe, expect, it, vi } from 'vitest';

import {
  createPostHogNativeTransport,
  PostHogNativeTransport,
  type PostHogNativeClient,
} from './index.js';

interface ProductEvent {
  name: 'onboarding_started';
  properties: { source: 'organic' | 'invite'; contact?: string };
}

const ANONYMOUS_ID = '00000000-0000-4000-8000-000000000001';
const USER_ID = '10000000-0000-4000-8000-000000000001';
const CAPTURED_AT = '2026-08-08T10:00:00.000Z';

const context: AnalyticsContext = {
  schema_version: 1,
  app: 'example-native',
  app_version: '1.0.0',
  platform: 'native',
  environment: 'test',
  locale: 'en-GB',
};

interface MockClient {
  readonly client: PostHogNativeClient;
  readonly capture: ReturnType<typeof vi.fn>;
  readonly identify: ReturnType<typeof vi.fn>;
  readonly alias: ReturnType<typeof vi.fn>;
  readonly reset: ReturnType<typeof vi.fn>;
}

function createMockClient(): MockClient {
  const capture = vi.fn();
  const identify = vi.fn();
  const alias = vi.fn();
  const reset = vi.fn();
  return { client: { capture, identify, alias, reset }, capture, identify, alias, reset };
}

function lifecycleEnvelopes(): readonly AnalyticsEnvelope<ProductEvent>[] {
  return [
    {
      type: 'capture',
      captured_at: CAPTURED_AT,
      anonymous_id: ANONYMOUS_ID,
      event: {
        ...context,
        name: 'onboarding_started',
        properties: { source: 'organic', contact: '[redacted]' },
      },
    },
    {
      type: 'identify',
      captured_at: CAPTURED_AT,
      anonymous_id: ANONYMOUS_ID,
      user_id: USER_ID,
      traits: { plan: 'free', email: '[redacted]' },
    },
    {
      type: 'alias',
      captured_at: CAPTURED_AT,
      anonymous_id: ANONYMOUS_ID,
      user_id: USER_ID,
    },
    {
      type: 'reset',
      captured_at: CAPTURED_AT,
      previous_anonymous_id: ANONYMOUS_ID,
      previous_user_id: USER_ID,
    },
  ];
}

describe('PostHogNativeTransport mapping', () => {
  it('maps capture, identify, alias, and reset without adding properties', async () => {
    const posthog = createMockClient();
    const transport = new PostHogNativeTransport<ProductEvent>(posthog.client);

    await transport.send(lifecycleEnvelopes());

    expect(posthog.capture).toHaveBeenCalledWith(
      'onboarding_started',
      {
        source: 'organic',
        contact: '[redacted]',
        ...context,
      },
      { timestamp: new Date(CAPTURED_AT) },
    );
    expect(posthog.identify).toHaveBeenCalledWith(USER_ID, {
      plan: 'free',
      email: '[redacted]',
    });
    expect(posthog.alias).toHaveBeenCalledWith(USER_ID);
    expect(posthog.reset).toHaveBeenCalledWith();
  });

  it('contains client and diagnostic failures', async () => {
    const posthog = createMockClient();
    posthog.capture.mockImplementation(() => {
      throw new Error('provider failed');
    });
    const transport = new PostHogNativeTransport<ProductEvent>(posthog.client, () => {
      throw new Error('diagnostic failed');
    });

    await expect(transport.send(lifecycleEnvelopes())).resolves.toBeUndefined();
    expect(posthog.identify).toHaveBeenCalledOnce();
  });
});

describe('createPostHogNativeTransport', () => {
  it('requires a self-hosted API host and API key', () => {
    expect(() => createPostHogNativeTransport({ apiKey: 'phc_key', apiHost: '' })).toThrow(
      'apiHost must not be empty',
    );
    expect(() =>
      createPostHogNativeTransport({ apiKey: 'phc_key', apiHost: 'posthog.internal' }),
    ).toThrow('apiHost must be an absolute http(s) URL');
    expect(() =>
      createPostHogNativeTransport({ apiKey: '', apiHost: 'https://posthog.example.test' }),
    ).toThrow('apiKey must not be empty');
  });

  it('initializes lazily with typed-only privacy defaults and the core anonymous ID', async () => {
    const posthog = createMockClient();
    const initializer = vi.fn(() => posthog.client);
    const transport = createPostHogNativeTransport<ProductEvent>({
      apiKey: ' phc_key ',
      apiHost: 'https://posthog.example.test/',
      initializer,
    });

    expect(initializer).not.toHaveBeenCalled();
    await transport.send(lifecycleEnvelopes().slice(0, 1));

    expect(initializer).toHaveBeenCalledWith(
      'phc_key',
      expect.objectContaining({
        host: 'https://posthog.example.test',
        bootstrap: { distinctId: ANONYMOUS_ID, isIdentifiedId: false },
        captureAppLifecycleEvents: false,
        capturePushNotificationOpened: false,
        capturePushNotificationSubscriptions: false,
        disableSurveys: true,
        enableSessionReplay: false,
        errorTracking: { autocapture: false },
        personProfiles: 'identified_only',
        preloadFeatureFlags: false,
        setDefaultPersonProperties: false,
      }),
    );
  });
});

describe('native adapter composed with analytics core', () => {
  it('drops before consent and transports only allowlisted, scrubbed properties after opt-in', async () => {
    const posthog = createMockClient();
    const allowlist = {
      onboarding_started: ['source', 'contact'],
    } as const satisfies EventAllowlist<ProductEvent>;
    const analytics = new AnalyticsClient<ProductEvent>({
      context,
      allowlist,
      transport: new PostHogNativeTransport(posthog.client),
      uuidFactory: () => ANONYMOUS_ID,
      flushBatchSize: 100,
      flushIntervalMs: 60_000,
      development: false,
    });

    analytics.capture({ name: 'onboarding_started', properties: { source: 'organic' } });
    await analytics.flush();
    analytics.setConsent('denied');
    analytics.capture({ name: 'onboarding_started', properties: { source: 'invite' } });
    await analytics.flush();
    expect(posthog.capture).not.toHaveBeenCalled();

    analytics.setConsent('granted');
    analytics.capture({
      name: 'onboarding_started',
      properties: {
        source: 'organic',
        contact: 'person@example.com',
        unlisted: 'must not arrive',
      },
    } as ProductEvent);
    await analytics.flush();

    expect(posthog.capture).toHaveBeenCalledOnce();
    expect(posthog.capture).toHaveBeenCalledWith(
      'onboarding_started',
      {
        source: 'organic',
        contact: '[redacted]',
        ...context,
      },
      expect.any(Object),
    );
  });
});
