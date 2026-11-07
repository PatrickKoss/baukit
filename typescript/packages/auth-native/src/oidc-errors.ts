export type OidcErrorCode =
  | 'authorization_failed'
  | 'callback_state_mismatch'
  | 'discovery_failed'
  | 'invalid_discovery_document'
  | 'invalid_token_response'
  | 'invalid_userinfo_response'
  | 'issuer_mismatch'
  | 'pkce_verifier_missing'
  | 'refresh_failed'
  | 'storage_failed'
  | 'token_exchange_failed'
  | 'userinfo_failed';

const SAFE_ERROR_MESSAGES = {
  authorization_failed: 'OIDC authorization failed.',
  callback_state_mismatch: 'OIDC callback state did not match the login transaction.',
  discovery_failed: 'OIDC provider discovery failed.',
  invalid_discovery_document: 'OIDC provider returned invalid discovery metadata.',
  invalid_token_response: 'OIDC token endpoint returned an invalid response.',
  invalid_userinfo_response: 'OIDC UserInfo endpoint returned an invalid response.',
  issuer_mismatch: 'OIDC provider issuer did not match the configured issuer.',
  pkce_verifier_missing: 'OIDC login did not produce a PKCE verifier.',
  refresh_failed: 'OIDC token refresh failed.',
  storage_failed: 'Secure authentication storage failed.',
  token_exchange_failed: 'OIDC token exchange failed.',
  userinfo_failed: 'OIDC UserInfo request failed.',
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
