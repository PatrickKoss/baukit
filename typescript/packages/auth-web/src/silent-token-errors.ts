export type SilentTokenErrorCode =
  | 'invalid_product'
  | 'invalid_account_binding'
  | 'invalid_url'
  | 'unsupported_environment'
  | 'unknown_product_or_audience'
  | 'disposed'
  | 'popup_blocked'
  | 'aborted'
  | 'invalid_callback'
  | 'invalid_state'
  | 'invalid_issuer'
  | 'authorization_rejected'
  | 'missing_code'
  | 'invalid_tokens'
  | 'invalid_nonce'
  | 'invalid_client'
  | 'invalid_metadata'
  | 'http_error'
  | 'invalid_response'
  | 'unavailable_or_invalid_response';

/** Contains only library-owned codes, never provider content or tokens. */
export class SilentTokenError extends Error {
  public constructor(public readonly code: SilentTokenErrorCode) {
    super(`Silent authorization failed: ${code}.`);
    this.name = 'SilentTokenError';
  }
}
