import type { AnalyticsEnvelope, AnalyticsEvent, Transport } from './types.js';

/** A production-safe transport that intentionally discards every command. */
export class NoopTransport<E extends AnalyticsEvent = AnalyticsEvent> implements Transport<E> {
  public send(envelopes: readonly AnalyticsEnvelope<E>[]): void {
    void envelopes;
  }
}

/** An inspectable transport intended for unit tests and local development. */
export class InMemoryTransport<E extends AnalyticsEvent = AnalyticsEvent> implements Transport<E> {
  readonly #envelopes: AnalyticsEnvelope<E>[] = [];

  public get envelopes(): readonly AnalyticsEnvelope<E>[] {
    return [...this.#envelopes];
  }

  public send(envelopes: readonly AnalyticsEnvelope<E>[]): void {
    this.#envelopes.push(...envelopes);
  }

  public clear(): void {
    this.#envelopes.length = 0;
  }
}
