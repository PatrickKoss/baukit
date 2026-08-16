import { describe, expect, it, vi } from 'vitest';
import { InMemoryScopedPersistenceRegistryStore } from '@baukit/data-contracts';

import { createProductPersistenceLifecycle } from './persistence-lifecycle';

const digest = (value: string): Promise<string> =>
  Promise.resolve(
    value.endsWith(':account-e') ? 'e'.repeat(64) : 'f'.repeat(64),
  );

describe('authenticated cache composition', () => {
  it('closes and resets before a different subject becomes ready', async () => {
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

  it('closes and blocks instead of switching on terminal expiry', async () => {
    const close = vi.fn(() => Promise.resolve());
    const lifecycle = createProductPersistenceLifecycle({
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: () => Promise.resolve({ close }),
      resetUserScopedState: () => undefined,
    });
    await lifecycle.selectSubject('account-e');

    await lifecycle.handleSessionExpired();

    expect(close).toHaveBeenCalledOnce();
    expect(lifecycle.current()).toBeUndefined();
    expect(lifecycle.state).toMatchObject({
      status: 'blocked',
      reason: 'session-expired',
    });
  });
});
