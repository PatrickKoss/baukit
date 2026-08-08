import { scrubProperties } from './scrubber.js';
import { InMemoryAnalyticsStorage } from './storage.js';
import { NoopTransport } from './transports.js';
import type {
  AliasEnvelope,
  AnalyticsClientOptions,
  AnalyticsContext,
  AnalyticsEnvelope,
  AnalyticsEvent,
  AnalyticsPort,
  AnalyticsStorage,
  ConsentState,
  EventAllowlist,
  IdentifyEnvelope,
  ResetEnvelope,
  SafeTraits,
  Transport,
  TransportFailure,
} from './types.js';

const DEFAULT_MAX_QUEUE_SIZE = 100;
const DEFAULT_FLUSH_BATCH_SIZE = 20;
const DEFAULT_FLUSH_INTERVAL_MS = 5_000;
const DEFAULT_MAX_RETRIES = 2;
const DEFAULT_RETRY_DELAY_MS = 100;
const UUID_PATTERN = /^[A-Fa-f0-9]{8}-(?:[A-Fa-f0-9]{4}-){3}[A-Fa-f0-9]{12}$/;
const IRREGULAR_PAST_TENSE = new Set([
  'begun',
  'bought',
  'built',
  'caught',
  'chosen',
  'done',
  'found',
  'given',
  'gone',
  'kept',
  'known',
  'left',
  'lost',
  'made',
  'paid',
  'read',
  'run',
  'seen',
  'sent',
  'shown',
  'sold',
  'taken',
  'won',
  'written',
]);

interface TimeoutRuntime {
  setTimeout(callback: () => void, milliseconds: number): unknown;
  clearTimeout(handle: unknown): void;
}

interface NodeStyleTimeout {
  unref(): void;
}

function getTimeoutRuntime(): TimeoutRuntime {
  const runtime = globalThis as unknown as Partial<TimeoutRuntime>;
  if (runtime.setTimeout === undefined || runtime.clearTimeout === undefined) {
    throw new Error('@baukit/analytics-core requires setTimeout and clearTimeout');
  }
  return runtime as TimeoutRuntime;
}

function inferDevelopmentMode(): boolean {
  const runtime = globalThis as unknown as {
    process?: { env?: Readonly<Record<string, string | undefined>> };
  };
  return runtime.process?.env?.['NODE_ENV'] !== 'production';
}

function defaultWarning(message: string): void {
  const runtime = globalThis as unknown as { console?: { warn(value: string): void } };
  runtime.console?.warn(message);
}

function defaultUuidFactory(): string {
  const runtime = globalThis as unknown as { crypto?: { randomUUID?: () => string } };
  if (runtime.crypto?.randomUUID === undefined) {
    throw new Error(
      '@baukit/analytics-core requires crypto.randomUUID; provide uuidFactory on runtimes without it',
    );
  }
  return runtime.crypto.randomUUID();
}

function requireInteger(name: string, value: number, minimum: number): number {
  if (!Number.isInteger(value) || value < minimum) {
    throw new TypeError(`${name} must be an integer greater than or equal to ${String(minimum)}`);
  }
  return value;
}

function requireNonEmpty(name: string, value: string): string {
  if (value.trim().length === 0) {
    throw new TypeError(`${name} must not be empty`);
  }
  return value;
}

function validateContext(context: AnalyticsContext): AnalyticsContext {
  requireInteger('context.schema_version', context.schema_version, 1);
  requireNonEmpty('context.app', context.app);
  requireNonEmpty('context.app_version', context.app_version);
  requireNonEmpty('context.platform', context.platform);
  requireNonEmpty('context.environment', context.environment);
  requireNonEmpty('context.locale', context.locale);
  return { ...context };
}

function isUuid(value: string): boolean {
  return UUID_PATTERN.test(value);
}

/** A deliberately conservative development-time heuristic, not a type-level grammar. */
export function followsEventNameConvention(name: string): boolean {
  if (!/^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/.test(name)) {
    return false;
  }
  return name.split('_').some((part) => part.endsWith('ed') || IRREGULAR_PAST_TENSE.has(part));
}

export class AnalyticsClient<E extends AnalyticsEvent> implements AnalyticsPort<E> {
  readonly #context: AnalyticsContext;
  readonly #allowlist: EventAllowlist<E>;
  readonly #transport: Transport<E>;
  readonly #storage: AnalyticsStorage;
  readonly #storagePrefix: string;
  readonly #uuidFactory: () => string;
  readonly #blockedKeys: readonly string[];
  readonly #maxQueueSize: number;
  readonly #flushBatchSize: number;
  readonly #flushIntervalMs: number;
  readonly #maxRetries: number;
  readonly #retryDelayMs: number;
  readonly #development: boolean;
  readonly #now: () => Date;
  readonly #onWarning: (message: string) => void;
  readonly #onTransportFailure: ((failure: TransportFailure<E>) => void) | undefined;
  readonly #queue: AnalyticsEnvelope<E>[] = [];
  readonly #warnedEventNames = new Set<string>();
  #consent: ConsentState;
  #anonymousId: string;
  #userId: string | undefined;
  #aliasedUserId: string | undefined;
  #flushTimer: unknown;
  #immediateFlushScheduled = false;
  #activeFlush: Promise<void> | undefined;

  public constructor(options: AnalyticsClientOptions<E>) {
    this.#context = validateContext(options.context);
    this.#allowlist = options.allowlist;
    this.#transport = options.transport ?? new NoopTransport<E>();
    this.#storage = options.storage ?? new InMemoryAnalyticsStorage();
    this.#storagePrefix = options.storageKeyPrefix ?? `@baukit/analytics-core:${this.#context.app}`;
    this.#uuidFactory = options.uuidFactory ?? defaultUuidFactory;
    this.#blockedKeys = [...(options.blockedKeys ?? [])];
    this.#maxQueueSize = requireInteger(
      'maxQueueSize',
      options.maxQueueSize ?? DEFAULT_MAX_QUEUE_SIZE,
      1,
    );
    this.#flushBatchSize = requireInteger(
      'flushBatchSize',
      options.flushBatchSize ?? Math.min(DEFAULT_FLUSH_BATCH_SIZE, this.#maxQueueSize),
      1,
    );
    this.#flushIntervalMs = requireInteger(
      'flushIntervalMs',
      options.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS,
      1,
    );
    this.#maxRetries = requireInteger('maxRetries', options.maxRetries ?? DEFAULT_MAX_RETRIES, 0);
    this.#retryDelayMs = requireInteger(
      'retryDelayMs',
      options.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS,
      0,
    );
    this.#development = options.development ?? inferDevelopmentMode();
    this.#now = options.now ?? (() => new Date());
    this.#onWarning = options.onWarning ?? defaultWarning;
    this.#onTransportFailure = options.onTransportFailure;

    const storedConsent = this.#readStorage(this.#consentKey);
    this.#consent =
      storedConsent === 'granted' || storedConsent === 'denied' || storedConsent === 'unknown'
        ? storedConsent
        : 'unknown';

    const storedAnonymousId = this.#readStorage(this.#anonymousIdKey);
    this.#anonymousId =
      storedAnonymousId !== undefined && isUuid(storedAnonymousId)
        ? storedAnonymousId
        : this.#generateAnonymousId();
    this.#writeStorage(this.#anonymousIdKey, this.#anonymousId);

    const storedUserId = this.#readStorage(this.#userIdKey);
    this.#userId = storedUserId !== undefined && isUuid(storedUserId) ? storedUserId : undefined;

    const storedAliasedUserId = this.#readStorage(this.#aliasedUserIdKey);
    this.#aliasedUserId =
      storedAliasedUserId !== undefined && isUuid(storedAliasedUserId)
        ? storedAliasedUserId
        : undefined;
  }

  public get consent(): ConsentState {
    return this.#consent;
  }

  public get anonymousId(): string {
    return this.#anonymousId;
  }

  public get userId(): string | undefined {
    return this.#userId;
  }

  public get pendingCount(): number {
    return this.#queue.length;
  }

  public capture(event: E): void {
    if (this.#consent !== 'granted') {
      return;
    }

    this.#warnAboutEventName(event.name);
    const allowedProperties = this.#allowedProperties(event);
    if (allowedProperties === undefined) {
      this.#warn(`Dropped undeclared analytics event "${event.name}".`);
      return;
    }

    const filtered: Record<string, unknown> = {};
    const source = event.properties as Readonly<Record<string, unknown>>;
    for (const propertyName of allowedProperties) {
      if (Object.prototype.hasOwnProperty.call(source, propertyName)) {
        filtered[propertyName] = source[propertyName];
      }
    }

    const envelope: AnalyticsEnvelope<E> = {
      type: 'capture',
      captured_at: this.#timestamp(),
      anonymous_id: this.#anonymousId,
      ...(this.#userId === undefined ? {} : { user_id: this.#userId }),
      event: {
        ...this.#context,
        name: event.name,
        properties: scrubProperties(filtered, { blockedKeys: this.#blockedKeys }),
      },
    };
    this.#enqueue(envelope);
  }

  public identify(userId: string, traits?: SafeTraits): void {
    if (this.#consent !== 'granted') {
      return;
    }
    if (!isUuid(userId)) {
      this.#warn('Dropped analytics identify call because userId is not a UUID.');
      return;
    }
    if (this.#userId !== undefined && this.#userId !== userId) {
      this.#warn('Dropped analytics identify call for a different user; call reset() first.');
      return;
    }

    this.#userId = userId;
    this.#writeStorage(this.#userIdKey, userId);
    const scrubbedTraits =
      traits === undefined
        ? undefined
        : scrubProperties(traits, {
            blockedKeys: this.#blockedKeys,
          });
    const envelope: IdentifyEnvelope = {
      type: 'identify',
      captured_at: this.#timestamp(),
      anonymous_id: this.#anonymousId,
      user_id: userId,
      ...(scrubbedTraits === undefined ? {} : { traits: scrubbedTraits }),
    };
    this.#enqueue(envelope);
  }

  public alias(anonymousId: string, userId: string): void {
    if (this.#consent !== 'granted') {
      return;
    }
    if (!isUuid(anonymousId) || anonymousId !== this.#anonymousId) {
      this.#warn('Dropped analytics alias call because anonymousId is not the current UUID.');
      return;
    }
    if (!isUuid(userId) || this.#userId !== userId) {
      this.#warn('Dropped analytics alias call; identify() must first set the same UUID userId.');
      return;
    }
    if (this.#aliasedUserId !== undefined) {
      this.#warn('Dropped repeated analytics alias call for the current anonymous identity.');
      return;
    }

    this.#aliasedUserId = userId;
    this.#writeStorage(this.#aliasedUserIdKey, userId);
    const envelope: AliasEnvelope = {
      type: 'alias',
      captured_at: this.#timestamp(),
      anonymous_id: anonymousId,
      user_id: userId,
    };
    this.#enqueue(envelope);
  }

  public reset(): void {
    const previousAnonymousId = this.#anonymousId;
    const previousUserId = this.#userId;
    const nextAnonymousId = this.#generateAnonymousId(previousAnonymousId);

    if (this.#consent === 'granted') {
      const envelope: ResetEnvelope = {
        type: 'reset',
        captured_at: this.#timestamp(),
        previous_anonymous_id: previousAnonymousId,
        ...(previousUserId === undefined ? {} : { previous_user_id: previousUserId }),
      };
      this.#enqueue(envelope);
    }

    this.#anonymousId = nextAnonymousId;
    this.#userId = undefined;
    this.#aliasedUserId = undefined;
    this.#writeStorage(this.#anonymousIdKey, nextAnonymousId);
    this.#removeStorage(this.#userIdKey);
    this.#removeStorage(this.#aliasedUserIdKey);
  }

  public setConsent(value: ConsentState): void {
    const transitionedToDenied = this.#consent !== 'denied' && value === 'denied';
    this.#consent = value;
    this.#writeStorage(this.#consentKey, value);
    if (value !== 'granted') {
      this.#queue.length = 0;
      this.#clearFlushTimer();
    }
    if (transitionedToDenied) {
      this.#clearTransportPending();
    }
  }

  /** Flushes accepted commands and always resolves, including on transport failure. */
  public flush(): Promise<void> {
    if (this.#activeFlush !== undefined) {
      return this.#activeFlush;
    }
    this.#clearFlushTimer();
    const operation = this.#drainQueue().finally(() => {
      if (this.#activeFlush === operation) {
        this.#activeFlush = undefined;
      }
      if (this.#queue.length > 0 && this.#consent === 'granted') {
        this.#armFlushTimer();
      }
    });
    this.#activeFlush = operation;
    return operation;
  }

  /** Stops the interval timer after a best-effort flush. */
  public async dispose(): Promise<void> {
    this.#clearFlushTimer();
    await this.flush();
    this.#clearFlushTimer();
  }

  get #consentKey(): string {
    return `${this.#storagePrefix}:consent`;
  }

  get #anonymousIdKey(): string {
    return `${this.#storagePrefix}:anonymous-id`;
  }

  get #userIdKey(): string {
    return `${this.#storagePrefix}:user-id`;
  }

  get #aliasedUserIdKey(): string {
    return `${this.#storagePrefix}:aliased-user-id`;
  }

  #allowedProperties(event: E): readonly string[] | undefined {
    const allowlist = this.#allowlist as Readonly<Record<string, readonly string[] | undefined>>;
    return allowlist[event.name];
  }

  #enqueue(envelope: AnalyticsEnvelope<E>): void {
    if (this.#consent !== 'granted') {
      return;
    }
    if (this.#queue.length === this.#maxQueueSize) {
      this.#queue.shift();
    }
    this.#queue.push(envelope);

    if (this.#queue.length >= this.#flushBatchSize) {
      this.#requestImmediateFlush();
    } else {
      this.#armFlushTimer();
    }
  }

  #requestImmediateFlush(): void {
    if (this.#immediateFlushScheduled) {
      return;
    }
    this.#immediateFlushScheduled = true;
    void Promise.resolve()
      .then(() => {
        this.#immediateFlushScheduled = false;
        return this.flush();
      })
      .catch(() => {
        // flush() is deliberately no-throw; this guard prevents future regressions leaking rejections.
      });
  }

  #armFlushTimer(): void {
    if (this.#flushTimer !== undefined || this.#queue.length === 0) {
      return;
    }
    const timers = getTimeoutRuntime();
    this.#flushTimer = timers.setTimeout(() => {
      this.#flushTimer = undefined;
      void this.flush();
    }, this.#flushIntervalMs);
    if (this.#isNodeStyleTimeout(this.#flushTimer)) {
      this.#flushTimer.unref();
    }
  }

  #clearFlushTimer(): void {
    if (this.#flushTimer === undefined) {
      return;
    }
    getTimeoutRuntime().clearTimeout(this.#flushTimer);
    this.#flushTimer = undefined;
  }

  #clearTransportPending(): void {
    try {
      const operation = this.#transport.clearPending?.();
      if (operation !== undefined) {
        void Promise.resolve(operation).catch((error: unknown) => {
          this.#reportTransportFailure({ error, envelopes: [], attempts: 0 });
        });
      }
    } catch (error: unknown) {
      this.#reportTransportFailure({ error, envelopes: [], attempts: 0 });
    }
  }

  #isNodeStyleTimeout(value: unknown): value is NodeStyleTimeout {
    return (
      typeof value === 'object' &&
      value !== null &&
      'unref' in value &&
      typeof (value as Partial<NodeStyleTimeout>).unref === 'function'
    );
  }

  async #drainQueue(): Promise<void> {
    try {
      while (this.#queue.length > 0 && this.#hasConsent()) {
        const batch = this.#queue.splice(0, this.#flushBatchSize);
        let lastError: unknown;
        let attempts = 0;

        while (attempts <= this.#maxRetries && this.#hasConsent()) {
          attempts += 1;
          try {
            await this.#transport.send(batch);
            lastError = undefined;
            break;
          } catch (error: unknown) {
            lastError = error;
            if (attempts <= this.#maxRetries && this.#hasConsent()) {
              await this.#retryDelay(attempts);
            }
          }
        }

        if (lastError !== undefined) {
          this.#reportTransportFailure({ error: lastError, envelopes: batch, attempts });
        }
      }
    } catch (error: unknown) {
      this.#reportTransportFailure({ error, envelopes: [], attempts: 0 });
    }
  }

  #hasConsent(): boolean {
    return this.#consent === 'granted';
  }

  async #retryDelay(attempt: number): Promise<void> {
    const delay = this.#retryDelayMs * 2 ** (attempt - 1);
    if (delay === 0) {
      await Promise.resolve();
      return;
    }
    await new Promise<void>((resolve) => {
      getTimeoutRuntime().setTimeout(resolve, delay);
    });
  }

  #reportTransportFailure(failure: TransportFailure<E>): void {
    try {
      this.#onTransportFailure?.(failure);
    } catch {
      // Diagnostics are never allowed to affect application behavior.
    }
  }

  #timestamp(): string {
    try {
      return this.#now().toISOString();
    } catch {
      return new Date().toISOString();
    }
  }

  #generateAnonymousId(previous?: string): string {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const candidate = this.#uuidFactory();
      if (isUuid(candidate) && candidate !== previous) {
        return candidate;
      }
    }
    throw new Error('uuidFactory must return a fresh UUID');
  }

  #warnAboutEventName(name: string): void {
    if (
      !this.#development ||
      this.#warnedEventNames.has(name) ||
      followsEventNameConvention(name)
    ) {
      return;
    }
    this.#warnedEventNames.add(name);
    this.#warn(
      `Analytics event "${name}" does not appear to use the past-tense snake_case convention.`,
    );
  }

  #readStorage(key: string): string | undefined {
    try {
      return this.#storage.getItem(key);
    } catch {
      this.#warn(`Analytics storage read failed for "${key}"; using a privacy-safe default.`);
      return undefined;
    }
  }

  #writeStorage(key: string, value: string): void {
    try {
      this.#storage.setItem(key, value);
    } catch {
      this.#warn(`Analytics storage write failed for "${key}".`);
    }
  }

  #removeStorage(key: string): void {
    try {
      this.#storage.removeItem(key);
    } catch {
      this.#warn(`Analytics storage removal failed for "${key}".`);
    }
  }

  #warn(message: string): void {
    try {
      this.#onWarning(message);
    } catch {
      // Warnings are advisory and never alter analytics behavior.
    }
  }
}
