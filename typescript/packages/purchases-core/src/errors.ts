export type PurchaseErrorCode =
  | 'cancelled'
  | 'pending'
  | 'network'
  | 'store'
  | 'unavailable'
  | 'not_identified'
  | 'identity_changed'
  | 'subject_mismatch'
  | 'invalid_subject'
  | 'not_credit_pack'
  | 'busy';

export class PurchaseError extends Error {
  constructor(
    readonly code: PurchaseErrorCode,
    options?: { readonly cause?: unknown },
  ) {
    super(code, options);
    this.name = 'PurchaseError';
  }
}

export function isPurchaseError(error: unknown, code?: PurchaseErrorCode): error is PurchaseError {
  return error instanceof PurchaseError && (code === undefined || error.code === code);
}
