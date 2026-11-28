import { isTerminalRefreshError, type SignInResult } from '@baukit/auth-native';

/** Converts observable cancellation into non-error copy suitable for a live region. */
export function signInFeedback(result: SignInResult): string | undefined {
  return result.status === 'cancelled' ? 'Sign in cancelled. You can try again.' : undefined;
}

/** A renewal failure reaches the UI once it is terminal or the current token has expired. */
export function shouldReportRenewalError(cause: unknown, expiresAt: number, now: number): boolean {
  return isTerminalRefreshError(cause) || now >= expiresAt;
}
