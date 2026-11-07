import { OidcError, safeAuthErrorMessage } from '@baukit/auth-web';

export type SessionFailure =
  | { readonly kind: 'retry' }
  | { readonly kind: 'error'; readonly message: string };

/** A retryable sign-in error keeps the account and offers a retry; anything else is shown. */
export function sessionFailure(cause: unknown): SessionFailure {
  if (cause instanceof OidcError && cause.retryable) {
    return { kind: 'retry' };
  }
  return { kind: 'error', message: safeAuthErrorMessage(cause) };
}
