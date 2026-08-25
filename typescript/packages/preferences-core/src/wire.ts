export type OptionalWireValue<T> =
  | { readonly state: 'absent' }
  | { readonly state: 'null' }
  | { readonly state: 'value'; readonly value: T };

export function decodeOptionalWireValue<T>(
  payload: unknown,
  key: string,
  decodeValue: (value: unknown) => T,
): OptionalWireValue<T> {
  if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) {
    throw new TypeError('Optional wire payload must be an object');
  }
  if (!Object.hasOwn(payload, key)) {
    return { state: 'absent' };
  }
  const value = (payload as Record<string, unknown>)[key];
  return value === null ? { state: 'null' } : { state: 'value', value: decodeValue(value) };
}

export function encodeOptionalWireValue<TKey extends string, TValue>(
  key: TKey,
  value: OptionalWireValue<TValue>,
): Partial<Record<TKey, TValue | null>> {
  switch (value.state) {
    case 'absent':
      return {};
    case 'null':
      return { [key]: null } as Record<TKey, null>;
    case 'value':
      return { [key]: value.value } as Record<TKey, TValue>;
  }
}
