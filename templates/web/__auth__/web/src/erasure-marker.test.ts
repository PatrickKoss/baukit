// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InMemoryScopedPersistenceRegistryStore } from '@baukit/data-contracts';

import { profileErasureMarker } from './erasure-marker';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

beforeEach(() => {
  localStorage.clear();
});

describe('profile erasure marker', () => {
  it('finishes an interrupted local wipe on the next selection instead of opening', async () => {
    await profileErasureMarker.mark('account-e');
    const open = vi.fn(() => Promise.resolve({ close: () => Promise.resolve() }));
    const erase = vi.fn(() => Promise.resolve());
    const lifecycle = createProductPersistenceLifecycle({
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest: () => Promise.resolve('e'.repeat(64)),
      erasureMarker: profileErasureMarker,
      resetUserScopedState: () => undefined,
      open,
      erase,
    });

    await lifecycle.selectSubject('account-e');

    expect(open).not.toHaveBeenCalled();
    expect(erase).toHaveBeenCalledOnce();
    expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'profile-erased' });
    await expect(profileErasureMarker.isMarked('account-e')).resolves.toBe(false);
  });

  it('keeps markers per subject', async () => {
    await profileErasureMarker.mark('u1');
    await expect(profileErasureMarker.isMarked('u10')).resolves.toBe(false);
    await expect(profileErasureMarker.isMarked('u1')).resolves.toBe(true);
  });
});
