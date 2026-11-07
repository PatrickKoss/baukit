/** Stable blocking failure for corrupt ownership metadata or a subject mismatch. */
export class PersistenceIdentityMismatchError extends Error {
  public override readonly name = 'PersistenceIdentityMismatchError';
  public readonly code = 'persistence_identity_mismatch' as const;

  public constructor(
    message = 'Local data belongs to a different authenticated identity.',
    options?: ErrorOptions,
  ) {
    super(message, options);
  }
}

export function isPersistenceIdentityMismatchError(
  cause: unknown,
): cause is PersistenceIdentityMismatchError {
  return (
    cause instanceof PersistenceIdentityMismatchError ||
    (typeof cause === 'object' &&
      cause !== null &&
      Reflect.get(cause, 'code') === 'persistence_identity_mismatch')
  );
}

export function requiredIdentityPart(value: string, label: string): string {
  if (value.trim().length === 0) {
    throw new PersistenceIdentityMismatchError(`${label} must not be empty.`);
  }
  return value;
}
