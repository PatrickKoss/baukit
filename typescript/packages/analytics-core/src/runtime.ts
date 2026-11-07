import type { AnalyticsContext, ConsentState } from './types.js';

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

export interface TimeoutRuntime {
  setTimeout(callback: () => void, milliseconds: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface NodeStyleTimeout {
  unref(): void;
}

// The lib targets ES2022 without DOM or Node types, so host globals are read as unknown and probed.
function readGlobal(name: string): unknown {
  return Reflect.get(globalThis, name);
}

function readProperty(owner: unknown, name: string): unknown {
  return typeof owner === 'object' && owner !== null ? Reflect.get(owner, name) : undefined;
}

export function getTimeoutRuntime(): TimeoutRuntime {
  const setTimer = readGlobal('setTimeout');
  const clearTimer = readGlobal('clearTimeout');
  if (typeof setTimer !== 'function' || typeof clearTimer !== 'function') {
    throw new Error('@baukit/analytics-core requires setTimeout and clearTimeout');
  }
  return {
    setTimeout: (callback, milliseconds): unknown =>
      Reflect.apply(setTimer, globalThis, [callback, milliseconds]),
    clearTimeout: (handle) => {
      Reflect.apply(clearTimer, globalThis, [handle]);
    },
  };
}

export function inferDevelopmentMode(): boolean {
  const env = readProperty(readGlobal('process'), 'env');
  return readProperty(env, 'NODE_ENV') !== 'production';
}

export function defaultWarning(message: string): void {
  const console = readGlobal('console');
  const warn = readProperty(console, 'warn');
  if (typeof warn === 'function') {
    Reflect.apply(warn, console, [message]);
  }
}

export function defaultUuidFactory(): string {
  const crypto = readGlobal('crypto');
  const randomUUID = readProperty(crypto, 'randomUUID');
  if (typeof randomUUID !== 'function') {
    throw new Error(
      '@baukit/analytics-core requires crypto.randomUUID; provide uuidFactory on runtimes without it',
    );
  }
  const uuid: unknown = Reflect.apply(randomUUID, crypto, []);
  if (typeof uuid !== 'string') {
    throw new Error('@baukit/analytics-core expected crypto.randomUUID to return a string');
  }
  return uuid;
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

export function validateContext(context: AnalyticsContext): AnalyticsContext {
  requireInteger('context.schema_version', context.schema_version, 1);
  requireNonEmpty('context.app', context.app);
  requireNonEmpty('context.app_version', context.app_version);
  requireNonEmpty('context.platform', context.platform);
  requireNonEmpty('context.environment', context.environment);
  requireNonEmpty('context.locale', context.locale);
  return { ...context };
}

export function isUuid(value: string): boolean {
  return UUID_PATTERN.test(value);
}

/** A deliberately conservative development-time heuristic, not a type-level grammar. */
export function followsEventNameConvention(name: string): boolean {
  if (!/^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/.test(name)) {
    return false;
  }
  return name.split('_').some((part) => part.endsWith('ed') || IRREGULAR_PAST_TENSE.has(part));
}

export interface ClientLimits {
  readonly maxQueueSize: number;
  readonly flushBatchSize: number;
  readonly flushIntervalMs: number;
  readonly maxRetries: number;
  readonly retryDelayMs: number;
}

export function resolveClientLimits(
  options: Partial<Record<keyof ClientLimits, number>>,
): ClientLimits {
  const maxQueueSize = requireInteger(
    'maxQueueSize',
    options.maxQueueSize ?? DEFAULT_MAX_QUEUE_SIZE,
    1,
  );
  return {
    maxQueueSize,
    flushBatchSize: requireInteger(
      'flushBatchSize',
      options.flushBatchSize ?? Math.min(DEFAULT_FLUSH_BATCH_SIZE, maxQueueSize),
      1,
    ),
    flushIntervalMs: requireInteger(
      'flushIntervalMs',
      options.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS,
      1,
    ),
    maxRetries: requireInteger('maxRetries', options.maxRetries ?? DEFAULT_MAX_RETRIES, 0),
    retryDelayMs: requireInteger('retryDelayMs', options.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS, 0),
  };
}

export function storedConsentState(value: string | undefined): ConsentState {
  return value === 'granted' || value === 'denied' || value === 'unknown' ? value : 'unknown';
}

export function storedUuid(value: string | undefined): string | undefined {
  return value !== undefined && isUuid(value) ? value : undefined;
}
