export type OidcErrorCode =
  | 'authorization_failed'
  | 'callback_state_mismatch'
  | 'discovery_failed'
  | 'invalid_discovery_document'
  | 'invalid_token_response'
  | 'issuer_mismatch'
  | 'logout_failed'
  | 'refresh_failed'
  | 'token_exchange_failed';

const SAFE_ERROR_MESSAGES = {
  authorization_failed: 'OIDC authorization failed.',
  callback_state_mismatch: 'OIDC callback state did not match the login transaction.',
  discovery_failed: 'OIDC provider discovery failed.',
  invalid_discovery_document: 'OIDC provider returned invalid discovery metadata.',
  invalid_token_response: 'The session endpoint returned an invalid response.',
  issuer_mismatch: 'OIDC provider issuer did not match the configured issuer.',
  logout_failed: 'Sign-out failed.',
  refresh_failed: 'OIDC token refresh failed.',
  token_exchange_failed: 'OIDC token exchange failed.',
} as const satisfies Record<OidcErrorCode, string>;

/** An authentication failure whose message is safe to display or log. */
export class OidcError extends Error {
  public readonly code: OidcErrorCode;
  public readonly status: number | undefined;
  /** True only when retrying the failed operation may succeed without signing in again. */
  public readonly retryable: boolean;

  public constructor(
    code: OidcErrorCode,
    options: { readonly retryable?: boolean; readonly status?: number } = {},
  ) {
    super(SAFE_ERROR_MESSAGES[code]);
    this.name = 'OidcError';
    this.code = code;
    this.status = options.status;
    this.retryable = options.retryable ?? false;
  }
}

/** Returns an allowlisted message without reflecting untrusted error content. */
export function safeAuthErrorMessage(cause: unknown): string {
  return cause instanceof OidcError ? SAFE_ERROR_MESSAGES[cause.code] : 'OIDC login failed.';
}
