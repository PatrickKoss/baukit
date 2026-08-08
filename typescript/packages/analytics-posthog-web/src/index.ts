import { posthog, type PostHogConfig } from 'posthog-js';

import type {
  AnalyticsEnvelope,
  AnalyticsEvent,
  CaptureEnvelope,
  Transport,
} from '@baukit/analytics-core';

export interface PostHogWebClient {
  capture(
    event: string,
    properties?: Readonly<Record<string, unknown>>,
    options?: { readonly timestamp?: Date },
  ): unknown;
  identify(distinctId: string, traits?: Readonly<Record<string, unknown>>): unknown;
  alias(alias: string, original?: string): unknown;
  reset(): unknown;
  opt_in_capturing(options?: { readonly captureEventName?: string | null | false }): unknown;
  opt_out_capturing(): unknown;
}

type GuardedPostHogWebOption =
  | 'advanced_disable_feature_flags_on_first_load'
  | 'api_host'
  | 'autocapture'
  | 'bootstrap'
  | 'capture_exceptions'
  | 'capture_pageleave'
  | 'capture_pageview'
  | 'disable_session_recording'
  | 'disable_surveys'
  | 'opt_out_capturing_by_default'
  | 'opt_out_persistence_by_default'
  | 'person_profiles'
  | 'save_campaign_params'
  | 'save_referrer';

export type PostHogWebInitOptions = Omit<Partial<PostHogConfig>, GuardedPostHogWebOption>;

export type PostHogWebInitializer = (
  apiKey: string,
  options: Partial<PostHogConfig>,
) => PostHogWebClient | Promise<PostHogWebClient>;

export interface PostHogWebTransportConfig {
  readonly apiKey: string;
  /** Absolute URL of the product's self-hosted PostHog instance. */
  readonly apiHost: string;
  readonly options?: PostHogWebInitOptions;
  readonly initializer?: PostHogWebInitializer;
  readonly onError?: (error: unknown) => void;
}

type ClientFactory = (
  firstEnvelope: AnalyticsEnvelope,
) => PostHogWebClient | Promise<PostHogWebClient>;

interface PendingQueueInternals {
  _queue?: unknown[];
}

interface PostHogWebInternals {
  __request_queue?: unknown[];
  _requestQueue?: PendingQueueInternals;
  _retryQueue?: PendingQueueInternals;
}

function clearArray(value: unknown[] | undefined): void {
  if (value !== undefined) {
    value.length = 0;
  }
}

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
  readonly distinctID: string;
  readonly isIdentifiedID: boolean;
} {
  if (envelope.type === 'capture') {
    return envelope.user_id === undefined
      ? { distinctID: envelope.anonymous_id, isIdentifiedID: false }
      : { distinctID: envelope.user_id, isIdentifiedID: true };
  }
  if (envelope.type === 'reset') {
    return envelope.previous_user_id === undefined
      ? { distinctID: envelope.previous_anonymous_id, isIdentifiedID: false }
      : { distinctID: envelope.previous_user_id, isIdentifiedID: true };
  }
  return { distinctID: envelope.anonymous_id, isIdentifiedID: false };
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

/** Translates privacy-checked core envelopes to a PostHog browser client. */
export class PostHogWebTransport<
  E extends AnalyticsEvent = AnalyticsEvent,
> implements Transport<E> {
  readonly #clientFactory: ClientFactory;
  readonly #onError: ((error: unknown) => void) | undefined;
  #client: Promise<PostHogWebClient> | undefined;
  #clearGeneration = 0;
  #clearRequired = false;
  #clearOperation: Promise<void> | undefined;
  #providerOptedOut = true;

  public constructor(client: PostHogWebClient | ClientFactory, onError?: (error: unknown) => void) {
    this.#clientFactory = typeof client === 'function' ? client : () => client;
    this.#onError = onError;
  }

  /** Provider failures are contained and never reject into application code. */
  public async send(envelopes: readonly AnalyticsEnvelope<E>[]): Promise<void> {
    if (envelopes.length === 0) {
      return;
    }

    const generation = this.#clearGeneration;
    let client: PostHogWebClient;
    try {
      client = await this.#getClient(envelopes[0] as AnalyticsEnvelope);
    } catch (error: unknown) {
      this.#reportError(error);
      return;
    }

    await this.#clearOperation;
    if (generation !== this.#clearGeneration) {
      return;
    }
    if (this.#clearRequired) {
      this.#purgeClient(client);
      if (generation !== this.#clearGeneration) {
        return;
      }
      this.#clearRequired = false;
    }
    if (!this.#optIn(client)) {
      return;
    }

    for (const envelope of envelopes) {
      if (generation !== this.#clearGeneration) {
        return;
      }
      try {
        await this.#dispatch(client, envelope);
      } catch (error: unknown) {
        this.#reportError(error);
      }
    }
  }

  /** Opts PostHog out and purges its in-memory batch and retry queues. */
  public clearPending(): Promise<void> {
    this.#clearGeneration += 1;
    this.#clearRequired = true;
    this.#providerOptedOut = true;
    const client = this.#client;
    if (client === undefined) {
      return Promise.resolve();
    }

    const generation = this.#clearGeneration;
    const operation = client
      .then((resolvedClient) => {
        this.#purgeClient(resolvedClient);
        if (generation === this.#clearGeneration) {
          this.#clearRequired = false;
        }
      })
      .catch((error: unknown) => {
        this.#reportError(error);
      })
      .finally(() => {
        if (this.#clearOperation === operation) {
          this.#clearOperation = undefined;
        }
      });
    this.#clearOperation = operation;
    return operation;
  }

  #getClient(firstEnvelope: AnalyticsEnvelope): Promise<PostHogWebClient> {
    this.#client ??= Promise.resolve(this.#clientFactory(firstEnvelope));
    return this.#client;
  }

  async #dispatch(client: PostHogWebClient, envelope: AnalyticsEnvelope<E>): Promise<void> {
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
        await Promise.resolve(client.alias(envelope.user_id, envelope.anonymous_id));
        break;
      case 'reset':
        await Promise.resolve(client.reset());
        break;
    }
  }

  #optIn(client: PostHogWebClient): boolean {
    if (!this.#providerOptedOut) {
      return true;
    }
    try {
      client.opt_in_capturing({ captureEventName: false });
      this.#providerOptedOut = false;
      return true;
    } catch (error: unknown) {
      this.#reportError(error);
      return false;
    }
  }

  #purgeClient(client: PostHogWebClient): void {
    try {
      client.opt_out_capturing();
    } catch (error: unknown) {
      this.#reportError(error);
    }

    const internals = client as PostHogWebClient & PostHogWebInternals;
    clearArray(internals.__request_queue);
    clearArray(internals._requestQueue?._queue);
    clearArray(internals._retryQueue?._queue);
  }

  #reportError(error: unknown): void {
    try {
      this.#onError?.(error);
    } catch {
      // Provider diagnostics must not affect the application journey.
    }
  }
}

/** Creates a lazy, safely configured PostHog browser transport. */
export function createPostHogWebTransport<E extends AnalyticsEvent = AnalyticsEvent>(
  config: PostHogWebTransportConfig,
): PostHogWebTransport<E> {
  const apiKey = requireNonEmpty('apiKey', config.apiKey);
  const apiHost = requireApiHost(config.apiHost);
  const initializer: PostHogWebInitializer =
    config.initializer ?? ((key, options) => posthog.init(key, options));

  return new PostHogWebTransport<E>((firstEnvelope) => {
    const options: Partial<PostHogConfig> = {
      ...config.options,
      api_host: apiHost,
      autocapture: false,
      capture_pageview: false,
      capture_pageleave: false,
      capture_exceptions: false,
      disable_session_recording: true,
      disable_surveys: true,
      advanced_disable_feature_flags_on_first_load: true,
      opt_out_capturing_by_default: true,
      opt_out_persistence_by_default: true,
      person_profiles: 'identified_only',
      save_campaign_params: false,
      save_referrer: false,
      bootstrap: initialIdentity(firstEnvelope),
    };
    return initializer(apiKey, options);
  }, config.onError);
}
