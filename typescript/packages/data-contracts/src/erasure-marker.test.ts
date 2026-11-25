import { describe, expect, it, vi } from 'vitest';

import { eraseProductProfile, type ProfileErasureMarkerStore } from './erasure.js';
import { InMemoryScopedPersistenceRegistryStore } from './identity.js';
import { ScopedPersistenceLifecycle } from './scoped-lifecycle.js';

class MemoryMarkers implements ProfileErasureMarkerStore {
  public readonly marked = new Set<string>();
  public mark(subject: string): Promise<void> {
    this.marked.add(subject);
    return Promise.resolve();
  }
  public isMarked(subject: string): Promise<boolean> {
    return Promise.resolve(this.marked.has(subject));
  }
  public clear(subject: string): Promise<void> {
    this.marked.delete(subject);
    return Promise.resolve();
  }
}

const digest = (value: string): Promise<string> =>
  Promise.resolve(value.endsWith(':account-a') ? 'a'.repeat(64) : 'b'.repeat(64));

function lifecycleWith(store: ProfileErasureMarkerStore) {
  const open = vi.fn(() => Promise.resolve({ close: () => Promise.resolve() }));
  const erase = vi.fn(() => Promise.resolve());
  const lifecycle = new ScopedPersistenceLifecycle({
    namespace: 'example',
    registry: new InMemoryScopedPersistenceRegistryStore(),
    digest,
    open,
    erase,
    erasureMarker: store,
    resetUserScopedState: () => undefined,
  });
  return { lifecycle, open, erase };
}

describe('durable erasure marker', () => {
  it('stays marked when the process dies between server and local erase', async () => {
    const markers = new MemoryMarkers();
    const crash = new Error('crash');
    const result = await eraseProductProfile({
      erasureMarker: { subject: 'account-a', store: markers },
      eraseServerProfile: () => {
        expect(markers.marked.has('account-a')).toBe(true);
        return Promise.resolve({ operationId: 'op', status: 'completed' });
      },
      eraseLocalPartition: () => Promise.reject(crash),
      signOut: () => Promise.resolve(),
    });
    expect(result.status).toBe('local-failure');
    expect(markers.marked.has('account-a')).toBe(true);
  });

  it('clears the marker after a successful local delete or a server failure', async () => {
    const markers = new MemoryMarkers();
    const base = {
      erasureMarker: { subject: 'account-a', store: markers },
      eraseLocalPartition: () => Promise.resolve(),
      signOut: () => Promise.resolve(),
    };
    await eraseProductProfile({
      ...base,
      eraseServerProfile: () => Promise.resolve({ operationId: 'op', status: 'completed' }),
    });
    expect(markers.marked.size).toBe(0);
    await eraseProductProfile({ ...base, eraseServerProfile: () => Promise.reject(new Error()) });
    expect(markers.marked.size).toBe(0);
  });

  it('finishes the local delete on startup instead of reopening the partition', async () => {
    const markers = new MemoryMarkers();
    await markers.mark('account-a');
    const { lifecycle, open, erase } = lifecycleWith(markers);

    await expect(lifecycle.selectSubject('account-a')).resolves.toBeUndefined();

    expect(open).not.toHaveBeenCalled();
    expect(erase).toHaveBeenCalledOnce();
    expect(markers.marked.size).toBe(0);
    expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'profile-erased' });
    await expect(lifecycle.selectSubject('account-b')).resolves.toMatchObject({
      subject: 'account-b',
    });
  });
});
