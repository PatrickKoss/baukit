import type { PostHogOptions } from 'posthog-react-native';

import type {
  AnalyticsEnvelope,
  AnalyticsEvent,
  CaptureEnvelope,
  Transport,
} from '@baukit/analytics-core';

/** Structural surface used so tests do not require a React Native runtime. */
export interface PostHogNativeClient {
  capture(
    event: string,
    properties?: Readonly<Record<string, unknown>>,
    options?: { readonly timestamp?: Date },
  ): unknown;
  identify(distinctId: string, traits?: Readonly<Record<string, unknown>>): unknown;
  alias(alias: string): unknown;
  reset(): unknown;
}

type GuardedPostHogNativeOption =
  | 'bootstrap'
  | 'captureAppLifecycleEvents'
  | 'capturePushNotificationOpened'
  | 'capturePushNotificationSubscriptions'
  | 'disableSurveys'
  | 'enableSessionReplay'
  | 'errorTracking'
  | 'host'
  | 'personProfiles'
  | 'preloadFeatureFlags'
  | 'setDefaultPersonProperties';

export type PostHogNativeInitOptions = Omit<PostHogOptions, GuardedPostHogNativeOption>;

export type PostHogNativeInitializer = (
  apiKey: string,
  options: PostHogOptions,
) => PostHogNativeClient | Promise<PostHogNativeClient>;

export interface PostHogNativeTransportConfig {
  readonly apiKey: string;
  /** Absolute URL of the product's self-hosted PostHog instance. */
  readonly apiHost: string;
  readonly options?: PostHogNativeInitOptions;
  readonly initializer?: PostHogNativeInitializer;
  readonly onError?: (error: unknown) => void;
}

type ClientFactory = (
  firstEnvelope: AnalyticsEnvelope,
) => PostHogNativeClient | Promise<PostHogNativeClient>;

function requireNonEmpty(name: string, value: string): string {
  const normalized = value.trim();
  if (normalized.length === 0) {
    throw new TypeError(`${name} must not be empty`);
  }
  return normalized;
}

function requireApiHost(value: string): string {
  const apiHost = requireNonEmpty('apiHost', value).replace(/\/+$/, '');
  if (!/^https?:\/\/[^\s/]+(?:\/.*)?$/u.test(apiHost)) {
    throw new TypeError('apiHost must be an absolute http(s) URL');
  }
  return apiHost;
}

function initialIdentity(envelope: AnalyticsEnvelope): {
  readonly distinctId: string;
  readonly isIdentifiedId: boolean;
} {
  if (envelope.type === 'capture') {
    return envelope.user_id === undefined
      ? { distinctId: envelope.anonymous_id, isIdentifiedId: false }
      : { distinctId: envelope.user_id, isIdentifiedId: true };
  }
  if (envelope.type === 'reset') {
    return envelope.previous_user_id === undefined
      ? { distinctId: envelope.previous_anonymous_id, isIdentifiedId: false }
      : { distinctId: envelope.previous_user_id, isIdentifiedId: true };
  }
  return { distinctId: envelope.anonymous_id, isIdentifiedId: false };
}

function captureProperties<E extends AnalyticsEvent>(
  envelope: CaptureEnvelope<E>,
): Readonly<Record<string, unknown>> {
  return {
    ...envelope.event.properties,
    schema_version: envelope.event.schema_version,
    app: envelope.event.app,
    app_version: envelope.event.app_version,
    platform: envelope.event.platform,
    environment: envelope.event.environment,
    locale: envelope.event.locale,
  };
}

async function defaultInitializer(
  apiKey: string,
  options: PostHogOptions,
): Promise<PostHogNativeClient> {
  const posthogModule = await import('posthog-react-native');
  return new posthogModule.PostHog(apiKey, options);
}

/** Translates privacy-checked core envelopes to a PostHog React Native client. */
export class PostHogNativeTransport<
  E extends AnalyticsEvent = AnalyticsEvent,
> implements Transport<E> {
  readonly #clientFactory: ClientFactory;
  readonly #onError: ((error: unknown) => void) | undefined;
  #client: Promise<PostHogNativeClient> | undefined;

  public constructor(
    client: PostHogNativeClient | ClientFactory,
    onError?: (error: unknown) => void,
  ) {
    this.#clientFactory = typeof client === 'function' ? client : () => client;
    this.#onError = onError;
  }

  /** Provider failures are contained and never reject into application code. */
  public async send(envelopes: readonly AnalyticsEnvelope<E>[]): Promise<void> {
    if (envelopes.length === 0) {
      return;
    }

    let client: PostHogNativeClient;
    try {
      client = await this.#getClient(envelopes[0] as AnalyticsEnvelope);
    } catch (error: unknown) {
      this.#reportError(error);
      return;
    }

    for (const envelope of envelopes) {
      try {
        await this.#dispatch(client, envelope);
      } catch (error: unknown) {
        this.#reportError(error);
      }
    }
  }

  #getClient(firstEnvelope: AnalyticsEnvelope): Promise<PostHogNativeClient> {
    this.#client ??= Promise.resolve(this.#clientFactory(firstEnvelope));
    return this.#client;
  }

  async #dispatch(client: PostHogNativeClient, envelope: AnalyticsEnvelope<E>): Promise<void> {
    switch (envelope.type) {
      case 'capture':
        await Promise.resolve(
          client.capture(envelope.event.name, captureProperties(envelope), {
            timestamp: new Date(envelope.captured_at),
          }),
        );
        break;
      case 'identify':
        await Promise.resolve(client.identify(envelope.user_id, envelope.traits));
        break;
      case 'alias':
        await Promise.resolve(client.alias(envelope.user_id));
        break;
      case 'reset':
        await Promise.resolve(client.reset());
        break;
    }
  }

  #reportError(error: unknown): void {
    try {
      this.#onError?.(error);
    } catch {
      // Provider diagnostics must not affect the application journey.
    }
  }
}

/** Creates a lazy, safely configured PostHog React Native transport. */
export function createPostHogNativeTransport<E extends AnalyticsEvent = AnalyticsEvent>(
  config: PostHogNativeTransportConfig,
): PostHogNativeTransport<E> {
  const apiKey = requireNonEmpty('apiKey', config.apiKey);
  const apiHost = requireApiHost(config.apiHost);
  const initializer = config.initializer ?? defaultInitializer;

  return new PostHogNativeTransport<E>((firstEnvelope) => {
    const options: PostHogOptions = {
      ...config.options,
      host: apiHost,
      bootstrap: initialIdentity(firstEnvelope),
      captureAppLifecycleEvents: false,
      capturePushNotificationOpened: false,
      capturePushNotificationSubscriptions: false,
      disableSurveys: true,
      enableSessionReplay: false,
      errorTracking: { autocapture: false },
      personProfiles: 'identified_only',
      preloadFeatureFlags: false,
      setDefaultPersonProperties: false,
    };
    return initializer(apiKey, options);
  }, config.onError);
}
