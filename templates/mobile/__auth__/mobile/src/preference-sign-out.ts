import type { SignOutResult } from '@baukit/auth-native';

export async function signOutWithPreferenceReset(options: {
  readonly resetPreferenceIdentity: () => Promise<void>;
  readonly signOut: () => Promise<SignOutResult | undefined>;
}): Promise<SignOutResult | undefined> {
  await options.resetPreferenceIdentity();
  return options.signOut();
}
