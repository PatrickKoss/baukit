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

export interface ScrubberOptions {
  readonly blockedKeys?: readonly string[];
}

const EMAIL_PATTERN = /(?:^|\s|[<(])[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+(?:$|\s|[>),.;:!?])/i;
const JWT_PATTERN = /^[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}$/;
const LONG_HEX_PATTERN = /^[A-Fa-f0-9]{32,}$/;
const LONG_BASE64_PATTERN = /^[A-Za-z0-9+/_-]{32,}={0,2}$/;

function normalizeKey(key: string): string {
  return key.toLowerCase().replaceAll(/[^a-z0-9]/g, '');
}

function createBlockedKeyList(extensions: readonly string[]): readonly string[] {
  return [...DEFAULT_BLOCKED_KEYS, ...extensions].map(normalizeKey).filter((key) => key.length > 0);
}

function isBlockedKey(key: string, blockedKeys: readonly string[]): boolean {
  const normalized = normalizeKey(key);
  return blockedKeys.some((blockedKey) => normalized.includes(blockedKey));
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

function scrubValue(
  value: unknown,
  blockedKeys: readonly string[],
  ancestors: WeakSet<object>,
): unknown {
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
    result = value.map((item) => scrubValue(item, blockedKeys, ancestors));
  } else if (
    Object.getPrototypeOf(value) === Object.prototype ||
    Object.getPrototypeOf(value) === null
  ) {
    result = scrubRecord(value as Readonly<Record<string, unknown>>, blockedKeys, ancestors);
  } else {
    result = REDACTED_VALUE;
  }

  ancestors.delete(value);
  return result;
}

function scrubRecord(
  properties: Readonly<Record<string, unknown>>,
  blockedKeys: readonly string[],
  ancestors: WeakSet<object>,
): Record<string, unknown> {
  const scrubbed: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(properties)) {
    scrubbed[key] = isBlockedKey(key, blockedKeys)
      ? REDACTED_VALUE
      : scrubValue(value, blockedKeys, ancestors);
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
  const blockedKeys = createBlockedKeyList(options.blockedKeys ?? []);
  const ancestors = new WeakSet();
  ancestors.add(properties);
  return scrubRecord(properties, blockedKeys, ancestors);
}
