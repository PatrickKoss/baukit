import { describe, expect, it, jest } from '@jest/globals';
import { InMemoryScopedPersistenceRegistryStore } from '@baukit/data-contracts';

import { createProductPersistenceLifecycle } from './persistence-lifecycle';

const digest = (value: string): Promise<string> =>
  Promise.resolve(
    value.endsWith(':account-e') ? 'e'.repeat(64) : 'f'.repeat(64),
  );

describe('authenticated persistence composition', () => {
  it('closes, resets, and opens the new subject in order', async () => {
    const events: string[] = [];
    const lifecycle = createProductPersistenceLifecycle({
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: ({ subject }) => {
        events.push(`open:${subject}`);
        return Promise.resolve({
          close: () => {
            events.push(`close:${subject}`);
            return Promise.resolve();
          },
        });
      },
      resetUserScopedState: () => {
        events.push('reset');
      },
    });

    await lifecycle.selectSubject('account-e');
    await lifecycle.selectSubject('account-f');

    expect(events).toEqual([
      'reset',
      'open:account-e',
      'close:account-e',
      'reset',
      'open:account-f',
    ]);
    expect(lifecycle.current()?.subject).toBe('account-f');
  });

  it('treats terminal session expiry as close and block', async () => {
    const close = jest.fn(() => Promise.resolve());
    const lifecycle = createProductPersistenceLifecycle({
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: () => Promise.resolve({ close }),
      resetUserScopedState: () => undefined,
    });
    await lifecycle.selectSubject('account-e');

    await lifecycle.handleSessionExpired();

    expect(close).toHaveBeenCalledTimes(1);
    expect(lifecycle.current()).toBeUndefined();
    expect(lifecycle.state).toMatchObject({
      status: 'blocked',
      reason: 'session-expired',
    });
  });
});
