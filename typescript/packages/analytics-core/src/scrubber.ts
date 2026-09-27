export const REDACTED_VALUE = '[redacted]' as const;

export const DEFAULT_BLOCKED_KEYS = [
  'email',
  'name',
  'token',
  'password',
  'authorization',
  'cookie',
  'phone',
  'address',
] as const;

export const DEFAULT_EXACT_BLOCKED_KEYS = [
  'ip',
  'ip_address',
  'remote_addr',
  'x_forwarded_for',
  'x_real_ip',
] as const;

export const ERROR_EVENT_BLOCKED_KEYS = [
  'headers',
  'data',
  'query_string',
  'body',
  'vars',
  'geo',
  'env',
] as const;

export const ERROR_EVENT_PRESERVED_KEYS = [
  'event_id',
  'trace_id',
  'span_id',
  'parent_span_id',
  'debug_id',
  'code_id',
  'filename',
  'abs_path',
  'function',
  'module',
] as const;

export interface ScrubberOptions {
  /** Terms redacted when a normalized key contains them. */
  readonly blockedKeys?: readonly string[];
  /** Keys redacted only when the normalized key equals them. */
  readonly exactBlockedKeys?: readonly string[];
}

interface ScrubRules {
  readonly blockedKeys: readonly string[];
  readonly exactBlockedKeys: ReadonlySet<string>;
  readonly preservedKeys: ReadonlySet<string>;
}

const VALUE_ONLY_RULES: ScrubRules = {
  blockedKeys: [],
  exactBlockedKeys: new Set(),
  preservedKeys: new Set(),
};

const EMAIL_PATTERN = /(?:^|\s|[<(])[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+(?:$|\s|[>),.;:!?])/i;
const JWT_PATTERN = /^[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}$/;
const LONG_HEX_PATTERN = /^[A-Fa-f0-9]{32,}$/;
const LONG_BASE64_PATTERN = /^[A-Za-z0-9+/_-]{32,}={0,2}$/;

function normalizeKey(key: string): string {
  return key.toLowerCase().replaceAll(/[^a-z0-9]/g, '');
}

function normalizeKeys(keys: readonly string[]): string[] {
  return keys.map(normalizeKey).filter((key) => key.length > 0);
}

function createRules(
  options: ScrubberOptions,
  extraExactKeys: readonly string[] = [],
  preservedKeys: readonly string[] = [],
): ScrubRules {
  return {
    blockedKeys: normalizeKeys([...DEFAULT_BLOCKED_KEYS, ...(options.blockedKeys ?? [])]),
    exactBlockedKeys: new Set(
      normalizeKeys([
        ...DEFAULT_EXACT_BLOCKED_KEYS,
        ...extraExactKeys,
        ...(options.exactBlockedKeys ?? []),
      ]),
    ),
    preservedKeys: new Set(normalizeKeys(preservedKeys)),
  };
}

function isBlockedKey(normalizedKey: string, rules: ScrubRules): boolean {
  return (
    rules.exactBlockedKeys.has(normalizedKey) ||
    rules.blockedKeys.some((blockedKey) => normalizedKey.includes(blockedKey))
  );
}

function isSensitiveString(value: string): boolean {
  const candidate = value.trim();
  return (
    EMAIL_PATTERN.test(candidate) ||
    JWT_PATTERN.test(candidate) ||
    LONG_HEX_PATTERN.test(candidate) ||
    LONG_BASE64_PATTERN.test(candidate)
  );
}

function scrubValue(value: unknown, rules: ScrubRules, ancestors: WeakSet<object>): unknown {
  if (typeof value === 'string') {
    return isSensitiveString(value) ? REDACTED_VALUE : value;
  }

  if (
    value === null ||
    typeof value === 'number' ||
    typeof value === 'boolean' ||
    typeof value === 'undefined'
  ) {
    return value;
  }

  if (typeof value !== 'object') {
    return REDACTED_VALUE;
  }

  if (ancestors.has(value)) {
    return REDACTED_VALUE;
  }

  ancestors.add(value);
  let result: unknown;

  if (Array.isArray(value)) {
    result = value.map((item) => scrubValue(item, rules, ancestors));
  } else if (
    Object.getPrototypeOf(value) === Object.prototype ||
    Object.getPrototypeOf(value) === null
  ) {
    result = scrubRecord(value as Readonly<Record<string, unknown>>, rules, ancestors);
  } else {
    result = REDACTED_VALUE;
  }

  ancestors.delete(value);
  return result;
}

function scrubEntry(
  key: string,
  value: unknown,
  rules: ScrubRules,
  ancestors: WeakSet<object>,
): unknown {
  const normalizedKey = normalizeKey(key);
  if (typeof value === 'string' && rules.preservedKeys.has(normalizedKey)) {
    return value;
  }
  return isBlockedKey(normalizedKey, rules) ? REDACTED_VALUE : scrubValue(value, rules, ancestors);
}

function scrubRecord(
  properties: Readonly<Record<string, unknown>>,
  rules: ScrubRules,
  ancestors: WeakSet<object>,
): Record<string, unknown> {
  const scrubbed: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(properties)) {
    scrubbed[key] = scrubEntry(key, value, rules, ancestors);
  }

  return scrubbed;
}

/**
 * Returns a new object with blocked keys and sensitive string values redacted.
 * Nested objects and arrays are traversed; the input is never mutated.
 */
export function scrubProperties(
  properties: Readonly<Record<string, unknown>>,
  options: ScrubberOptions = {},
): Readonly<Record<string, unknown>> {
  return scrubRoot(properties, createRules(options));
}

function scrubRoot(
  record: Readonly<Record<string, unknown>>,
  rules: ScrubRules,
): Record<string, unknown> {
  const ancestors = new WeakSet();
  ancestors.add(record);
  return scrubRecord(record, rules, ancestors);
}

/**
 * Scrubs a crash-report event, such as the object a Sentry `beforeSend` hook receives.
 *
 * Applies the property rules plus {@link ERROR_EVENT_BLOCKED_KEYS}. String values under
 * {@link ERROR_EVENT_PRESERVED_KEYS} stay intact so event and trace IDs and stack frames survive.
 * The top-level `sdk` object keeps its keys and only has its values checked.
 */
export function scrubErrorEvent<T extends object>(event: T, options: ScrubberOptions = {}): T {
  const record = event as Readonly<Record<string, unknown>>;
  const scrubbed = scrubRoot(
    record,
    createRules(options, ERROR_EVENT_BLOCKED_KEYS, ERROR_EVENT_PRESERVED_KEYS),
  );
  if ('sdk' in record) {
    scrubbed['sdk'] = scrubValue(record['sdk'], VALUE_ONLY_RULES, new WeakSet([record]));
  }
  return scrubbed as T;
}
