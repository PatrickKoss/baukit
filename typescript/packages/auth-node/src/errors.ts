export type AuthNodeErrorCode =
  | 'aborted'
  | 'cache_corrupt'
  | 'cache_lock_timeout'
  | 'cache_permission'
  | 'cache_read_failed'
  | 'cache_symlink'
  | 'cache_write_failed'
  | 'device_authorization_denied'
  | 'device_authorization_expired'
  | 'device_authorization_failed'
  | 'discovery_failed'
  | 'endpoint_policy_violation'
  | 'invalid_device_response'
  | 'invalid_discovery_document'
  | 'invalid_token_response'
  | 'issuer_mismatch'
  | 'login_timeout'
  | 'no_cached_session'
  | 'refresh_failed';

const SAFE_MESSAGES = {
  aborted: 'OIDC operation was cancelled.',
  cache_corrupt: 'The local authentication cache is invalid.',
  cache_lock_timeout: 'Timed out waiting for the local authentication cache lock.',
  cache_permission: 'The local authentication cache permissions are unsafe.',
  cache_read_failed: 'Could not read the local authentication cache.',
  cache_symlink: 'The local authentication cache must not use symbolic links.',
  cache_write_failed: 'Could not update the local authentication cache.',
  device_authorization_denied: 'OIDC device authorization was denied.',
  device_authorization_expired: 'OIDC device authorization expired.',
  device_authorization_failed: 'OIDC device authorization failed.',
  discovery_failed: 'OIDC provider discovery failed.',
  endpoint_policy_violation: 'OIDC endpoint did not satisfy the configured security policy.',
  invalid_device_response: 'OIDC provider returned an invalid device authorization response.',
  invalid_discovery_document: 'OIDC provider returned invalid discovery metadata.',
  invalid_token_response: 'OIDC provider returned an invalid token response.',
  issuer_mismatch: 'OIDC provider issuer did not match an allowed issuer.',
  login_timeout: 'OIDC device authorization timed out.',
  no_cached_session: 'No usable OIDC session is available.',
  refresh_failed: 'OIDC token refresh failed.',
} as const satisfies Record<AuthNodeErrorCode, string>;

/** An authentication failure whose message never includes tokens or provider response bodies. */
export class AuthNodeError extends Error {
  public readonly code: AuthNodeErrorCode;
  public readonly retryable: boolean;
  public readonly status: number | undefined;

  public constructor(
    code: AuthNodeErrorCode,
    options: { readonly retryable?: boolean; readonly status?: number } = {},
  ) {
    super(SAFE_MESSAGES[code]);
    this.name = 'AuthNodeError';
    this.code = code;
    this.retryable = options.retryable ?? false;
    this.status = options.status;
  }
}

export function safeAuthNodeErrorMessage(cause: unknown): string {
  return cause instanceof AuthNodeError ? SAFE_MESSAGES[cause.code] : 'OIDC authentication failed.';
}
