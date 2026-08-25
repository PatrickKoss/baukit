import type { SignInResult } from '@baukit/auth-native';

/** Converts observable cancellation into non-error copy suitable for a live region. */
export function signInFeedback(result: SignInResult): string | undefined {
  return result.status === 'cancelled' ? 'Sign in cancelled. You can try again.' : undefined;
}
