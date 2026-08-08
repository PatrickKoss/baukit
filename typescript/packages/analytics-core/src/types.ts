/** A product-owned, statically typed analytics event. */
export interface AnalyticsEvent {
  readonly name: string;
  readonly properties: object;
}

export type ConsentState = 'granted' | 'denied' | 'unknown';

/** Traits must be bounded product metadata, never user-provided free text or PII. */
export type SafeTraits = Readonly<Record<string, unknown>>;

export interface AnalyticsPort<E extends AnalyticsEvent> {
  capture(event: E): void;
  identify(userId: string, traits?: SafeTraits): void;
  alias(anonymousId: string, userId: string): void;
  reset(): void;
  setConsent(value: ConsentState): void;
}

/** Required metadata stamped onto every captured event. */
export interface AnalyticsContext {
  readonly schema_version: number;
  readonly app: string;
  readonly app_version: string;
  readonly platform: string;
  readonly environment: string;
  readonly locale: string;
}

type EventPropertyName<Event extends AnalyticsEvent> = Extract<keyof Event['properties'], string>;

/**
 * A complete per-event allowlist. Adding an event to a product union makes its
 * allowlist entry a compile-time requirement.
 */
export type EventAllowlist<E extends AnalyticsEvent> = {
  readonly [Name in E['name']]: readonly EventPropertyName<Extract<E, { name: Name }>>[];
};

interface BaseEnvelope {
  readonly captured_at: string;
}

export interface CaptureEnvelope<E extends AnalyticsEvent = AnalyticsEvent> extends BaseEnvelope {
  readonly type: 'capture';
  readonly anonymous_id: string;
  readonly user_id?: string;
  readonly event: AnalyticsContext & {
    readonly name: E['name'];
    readonly properties: Readonly<Record<string, unknown>>;
  };
}

export interface IdentifyEnvelope extends BaseEnvelope {
  readonly type: 'identify';
  readonly anonymous_id: string;
  readonly user_id: string;
  readonly traits?: SafeTraits;
}

export interface AliasEnvelope extends BaseEnvelope {
  readonly type: 'alias';
  readonly anonymous_id: string;
  readonly user_id: string;
}

export interface ResetEnvelope extends BaseEnvelope {
  readonly type: 'reset';
  readonly previous_anonymous_id: string;
  readonly previous_user_id?: string;
}

/** Provider-neutral commands delivered in ordering-preserving batches. */
export type AnalyticsEnvelope<E extends AnalyticsEvent = AnalyticsEvent> =
  CaptureEnvelope<E> | IdentifyEnvelope | AliasEnvelope | ResetEnvelope;

export interface Transport<E extends AnalyticsEvent = AnalyticsEvent> {
  send(envelopes: readonly AnalyticsEnvelope<E>[]): Promise<void> | void;
}

/** Synchronous by design so consent can be applied before the first capture call. */
export interface AnalyticsStorage {
  getItem(key: string): string | undefined;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface TransportFailure<E extends AnalyticsEvent = AnalyticsEvent> {
  readonly error: unknown;
  readonly envelopes: readonly AnalyticsEnvelope<E>[];
  readonly attempts: number;
}

export interface AnalyticsClientOptions<E extends AnalyticsEvent> {
  readonly context: AnalyticsContext;
  readonly allowlist: EventAllowlist<E>;
  readonly transport?: Transport<E>;
  readonly storage?: AnalyticsStorage;
  readonly storageKeyPrefix?: string;
  readonly uuidFactory?: () => string;
  readonly blockedKeys?: readonly string[];
  readonly maxQueueSize?: number;
  readonly flushBatchSize?: number;
  readonly flushIntervalMs?: number;
  readonly maxRetries?: number;
  readonly retryDelayMs?: number;
  readonly development?: boolean;
  readonly now?: () => Date;
  readonly onWarning?: (message: string) => void;
  readonly onTransportFailure?: (failure: TransportFailure<E>) => void;
}
