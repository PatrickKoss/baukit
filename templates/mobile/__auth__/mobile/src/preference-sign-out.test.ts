import { describe, expect, it, jest } from '@jest/globals';

import { signOutWithPreferenceReset } from './preference-sign-out';

describe('preference sign-out', () => {
  it('resets the preference identity before signing out', async () => {
    const events: string[] = [];
    const resetPreferenceIdentity = jest.fn(() => {
      events.push('preferences:reset');
      return Promise.resolve();
    });
    const signOut = jest.fn(() => {
      events.push('auth:sign-out');
      return Promise.resolve({ providerLogout: 'completed' } as const);
    });

    await expect(signOutWithPreferenceReset({ resetPreferenceIdentity, signOut })).resolves.toEqual(
      { providerLogout: 'completed' },
    );
    expect(events).toEqual(['preferences:reset', 'auth:sign-out']);
  });
});
