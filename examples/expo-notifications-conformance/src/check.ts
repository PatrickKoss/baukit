export interface Mismatch {
  readonly label: string;
  readonly actual: string;
  readonly expected: string;
}

/** Canonical JSON with sorted keys; `undefined` fields drop out as in Vitest's `toEqual`. */
export function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, entry: unknown) => {
    if (typeof entry === "number" && !Number.isFinite(entry)) {
      return `non-finite:${String(entry)}`;
    }
    if (entry === null || typeof entry !== "object" || Array.isArray(entry)) {
      return entry;
    }
    const record = entry as Record<string, unknown>;
    return Object.fromEntries(
      Object.keys(record)
        .sort()
        .map((key) => [key, record[key]]),
    );
  });
}

export function mismatch(
  label: string,
  actual: unknown,
  expected: unknown,
): Mismatch | null {
  const actualText = canonical(actual);
  const expectedText = canonical(expected);
  return actualText === expectedText
    ? null
    : { label, actual: actualText, expected: expectedText };
}

export function assertEqual(
  label: string,
  actual: unknown,
  expected: unknown,
): void {
  const found = mismatch(label, actual, expected);
  if (found !== null) {
    throw new Error(
      `${label}: expected ${found.expected}, got ${found.actual}`,
    );
  }
}
