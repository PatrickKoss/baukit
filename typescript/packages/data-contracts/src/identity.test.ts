import { describe, expect, it, vi } from 'vitest';

import {
  InMemoryScopedPersistenceRegistryStore,
  PersistenceIdentityMismatchError,
  ScopedPersistenceLifecycle,
  deriveScopedStoreName,
  recheckServerSubjectBeforeSyncAdoption,
  removeScopedPersistenceRegistryEntry,
  resolveScopedStore,
} from './identity.js';

const digest = (value: string): Promise<string> =>
  Promise.resolve(value.endsWith(':account-e') ? 'e'.repeat(64) : 'f'.repeat(64));

describe('scoped persistence registry', () => {
  it('derives stable opaque names from an unambiguous namespace and subject encoding', async () => {
    const first = await deriveScopedStoreName('example', 'account-e');
    expect(first).toMatch(/^baukit-scoped-v1-[0-9a-f]{64}$/u);
    await expect(deriveScopedStoreName('example', 'account-e')).resolves.toBe(first);
    await expect(deriveScopedStoreName('exam', 'ple:account-e')).resolves.not.toBe(first);
    await expect(deriveScopedStoreName('', 'account-e')).rejects.toBeInstanceOf(
      PersistenceIdentityMismatchError,
    );
    await expect(deriveScopedStoreName('example', '', digest)).rejects.toMatchObject({
      code: 'persistence_identity_mismatch',
    });
    await expect(
      deriveScopedStoreName('example', 'account-e', () => Promise.resolve('not-sha256')),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
  });

  it('keeps E -> F -> E mappings stable and serializes a one-time legacy claim', async () => {
    const registry = new InMemoryScopedPersistenceRegistryStore();
    const inspectLegacy = vi.fn(() =>
      Promise.resolve({
        exists: true,
        ownership: 'claimable',
      } as const),
    );
    const base = {
      namespace: 'example',
      registry,
      digest,
      legacyStoreName: 'legacy.db',
      inspectLegacy,
    } as const;

    const [accountE, accountF] = await Promise.all([
      resolveScopedStore({ ...base, subject: 'account-e' }),
      resolveScopedStore({ ...base, subject: 'account-f' }),
    ]);
    expect([accountE, accountF].filter((result) => result.claimedLegacy)).toHaveLength(1);
    expect(new Set([accountE.storeName, accountF.storeName]).size).toBe(2);
    await expect(resolveScopedStore({ ...base, subject: 'account-e' })).resolves.toEqual(accountE);
    await expect(resolveScopedStore({ ...base, subject: 'account-f' })).resolves.toEqual(accountF);
    expect(inspectLegacy).toHaveBeenCalledOnce();
  });

  it.each([
    [{ exists: false } as const, false],
    [{ exists: true, ownership: 'ambiguous' } as const, false],
    [{ exists: true, ownership: 'other-subject' } as const, false],
    [{ exists: true, ownership: 'claimable' } as const, true],
    [{ exists: true, ownership: 'current-subject' } as const, true],
  ])('applies explicit legacy ownership rules to %j', async (inspection, claimedLegacy) => {
    const result = await resolveScopedStore({
      namespace: `legacy-${inspection.exists ? inspection.ownership : 'missing'}`,
      subject: 'account-e',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      legacyStoreName: 'legacy.db',
      inspectLegacy: () => Promise.resolve(inspection),
    });
    expect(result.claimedLegacy).toBe(claimedLegacy);
    expect(result.storeName === 'legacy.db').toBe(claimedLegacy);
  });

  it.each([
    '{truncated',
    JSON.stringify({ version: 2, namespaces: [] }),
    JSON.stringify({ version: 1, namespaces: 'invalid' }),
    JSON.stringify({
      version: 1,
      namespaces: [
        {
          namespace: 'example',
          legacyClaimedBySubject: null,
          subjects: [{ subject: 'account-e', storeName: 'legacy.db', claimedLegacy: true }],
        },
      ],
    }),
    JSON.stringify({
      version: 1,
      namespaces: [
        {
          namespace: 'example',
          legacyClaimedBySubject: null,
          subjects: [
            {
              subject: 'account-e',
              storeName: `baukit-scoped-v1-${'e'.repeat(64)}`,
              claimedLegacy: false,
            },
            {
              subject: 'account-f',
              storeName: `baukit-scoped-v1-${'e'.repeat(64)}`,
              claimedLegacy: false,
            },
          ],
        },
      ],
    }),
  ])('fails closed for corrupt registry metadata', async (serialized) => {
    const inspectLegacy = vi.fn();
    await expect(
      resolveScopedStore({
        namespace: 'example',
        subject: 'account-e',
        registry: new InMemoryScopedPersistenceRegistryStore(serialized),
        digest,
        inspectLegacy,
        legacyStoreName: 'legacy.db',
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
    expect(inspectLegacy).not.toHaveBeenCalled();
  });

  it('rejects a claimed legacy mapping unless its configured name still matches', async () => {
    const serialized = JSON.stringify({
      version: 1,
      namespaces: [
        {
          namespace: 'example',
          legacyClaimedBySubject: 'account-e',
          subjects: [{ subject: 'account-e', storeName: 'forged.db', claimedLegacy: true }],
        },
      ],
    });
    const registry = new InMemoryScopedPersistenceRegistryStore(serialized);

    await expect(
      resolveScopedStore({ namespace: 'example', subject: 'account-e', registry, digest }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
    await expect(
      resolveScopedStore({
        namespace: 'example',
        subject: 'account-e',
        registry,
        digest,
        legacyStoreName: 'legacy.db',
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
  });

  it('rejects duplicate store ownership before recording a legacy claim', async () => {
    const registry = new InMemoryScopedPersistenceRegistryStore();
    const accountE = await resolveScopedStore({
      namespace: 'example',
      subject: 'account-e',
      registry,
      digest,
    });

    await expect(
      resolveScopedStore({
        namespace: 'example',
        subject: 'account-f',
        registry,
        digest,
        legacyStoreName: accountE.storeName,
        inspectLegacy: () => Promise.resolve({ exists: true, ownership: 'claimable' }),
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
  });

  it('rejects duplicate store ownership across registry namespaces', async () => {
    const serialized = JSON.stringify({
      version: 1,
      namespaces: [
        {
          namespace: 'first',
          legacyClaimedBySubject: 'account-e',
          subjects: [{ subject: 'account-e', storeName: 'legacy.db', claimedLegacy: true }],
        },
        {
          namespace: 'second',
          legacyClaimedBySubject: 'account-f',
          subjects: [{ subject: 'account-f', storeName: 'legacy.db', claimedLegacy: true }],
        },
      ],
    });

    await expect(
      resolveScopedStore({
        namespace: 'first',
        subject: 'account-e',
        registry: new InMemoryScopedPersistenceRegistryStore(serialized),
        digest,
        legacyStoreName: 'legacy.db',
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
  });
});

describe('scoped persistence lifecycle', () => {
  it('closes, deletes, and unregisters the active partition in order', async () => {
    const events: string[] = [];
    const registry = new InMemoryScopedPersistenceRegistryStore();
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry,
      digest,
      open: () =>
        Promise.resolve({
          close: () => {
            events.push('close');
            return Promise.resolve();
          },
        }),
      erase: ({ subject, storeName }) => {
        events.push(`erase:${subject}:${storeName}`);
        return Promise.resolve();
      },
      resetUserScopedState: () => {
        events.push('reset');
      },
    });
    const partition = await lifecycle.selectSubject('account-e');

    await expect(lifecycle.eraseActivePartition()).resolves.toBe(true);

    expect(events).toEqual([
      'reset',
      'close',
      'reset',
      `erase:account-e:${partition?.storeName ?? ''}`,
    ]);
    expect(lifecycle.current()).toBeUndefined();
    expect(lifecycle.state).toEqual({ status: 'signed-out' });
    await expect(registry.read()).resolves.toBe('{"version":1,"namespaces":[]}');
  });

  it('treats active-partition erasure without an active partition as a safe no-op', async () => {
    const erase = vi.fn(() => Promise.resolve());
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: () => Promise.resolve({ close: () => Promise.resolve() }),
      erase,
      resetUserScopedState: () => undefined,
    });

    await expect(lifecycle.eraseActivePartition()).resolves.toBe(false);
    expect(erase).not.toHaveBeenCalled();
    expect(lifecycle.state).toEqual({ status: 'signed-out' });
  });

  it('keeps registry ownership when physical partition deletion fails', async () => {
    const registry = new InMemoryScopedPersistenceRegistryStore();
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry,
      digest,
      open: () => Promise.resolve({ close: () => Promise.resolve() }),
      erase: () => Promise.reject(new Error('delete failed')),
      resetUserScopedState: () => undefined,
    });
    const partition = await lifecycle.selectSubject('account-e');

    await expect(lifecycle.eraseActivePartition()).rejects.toThrow('delete failed');
    await expect(
      resolveScopedStore({ namespace: 'example', subject: 'account-e', registry, digest }),
    ).resolves.toEqual({ storeName: partition?.storeName, claimedLegacy: false });
  });

  it('keeps a legacy-claim tombstone after erasing the claiming subject', async () => {
    const registry = new InMemoryScopedPersistenceRegistryStore();
    const inspectLegacy = vi.fn(() =>
      Promise.resolve({ exists: true, ownership: 'claimable' } as const),
    );
    const base = {
      namespace: 'example',
      registry,
      digest,
      legacyStoreName: 'legacy.db',
      inspectLegacy,
    };
    const lifecycle = new ScopedPersistenceLifecycle({
      ...base,
      open: () => Promise.resolve({ close: () => Promise.resolve() }),
      erase: () => Promise.resolve(),
      resetUserScopedState: () => undefined,
    });
    await expect(lifecycle.selectSubject('account-e')).resolves.toMatchObject({
      storeName: 'legacy.db',
    });

    await expect(lifecycle.eraseActivePartition()).resolves.toBe(true);
    await expect(
      removeScopedPersistenceRegistryEntry({
        namespace: 'example',
        subject: 'account-e',
        registry,
      }),
    ).resolves.toBe(false);

    await expect(resolveScopedStore({ ...base, subject: 'account-e' })).resolves.toEqual({
      storeName: `baukit-scoped-v1-${'e'.repeat(64)}`,
      claimedLegacy: false,
    });
    expect(inspectLegacy).toHaveBeenCalledOnce();
  });

  it('fails closed when registry removal encounters corrupt metadata', async () => {
    await expect(
      removeScopedPersistenceRegistryEntry({
        namespace: 'example',
        subject: 'account-e',
        registry: new InMemoryScopedPersistenceRegistryStore('{truncated'),
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
  });

  it('accepts a product compatibility resolver without weakening lifecycle ordering', async () => {
    const persistence = { close: vi.fn(() => Promise.resolve()) };
    const resolveStore = vi.fn(() =>
      Promise.resolve({ storeName: 'pre-v0.6-account-e', claimedLegacy: false }),
    );
    const open = vi.fn(() => Promise.resolve(persistence));
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry: new InMemoryScopedPersistenceRegistryStore('{corrupt'),
      digest,
      resolveStore,
      open,
      resetUserScopedState: () => undefined,
    });

    await expect(lifecycle.selectSubject('account-e')).resolves.toMatchObject({
      storeName: 'pre-v0.6-account-e',
      persistence,
    });
    expect(resolveStore).toHaveBeenCalledWith('account-e');
    expect(open).toHaveBeenCalledWith({
      subject: 'account-e',
      storeName: 'pre-v0.6-account-e',
    });
  });

  it('does not publish a partition until open and hydration finish', async () => {
    let finishOpen: (() => void) | undefined;
    const opening = new Promise<void>((resolve) => {
      finishOpen = resolve;
    });
    const persistence = { close: vi.fn(() => Promise.resolve()) };
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      resetUserScopedState: () => undefined,
      open: async () => {
        await opening;
        return persistence;
      },
    });

    const selected = lifecycle.selectSubject('account-e');
    expect(lifecycle.state).toEqual({ status: 'initializing', subject: 'account-e' });
    expect(lifecycle.current()).toBeUndefined();
    await Promise.resolve();
    finishOpen?.();
    await expect(selected).resolves.toMatchObject({ subject: 'account-e', persistence });
    expect(lifecycle.state.status).toBe('ready');
  });

  it('closes and blocks on terminal session expiry without selecting another subject', async () => {
    const close = vi.fn(() => Promise.resolve());
    const open = vi.fn(() => Promise.resolve({ close }));
    const reset = vi.fn();
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open,
      resetUserScopedState: reset,
    });
    await lifecycle.selectSubject('account-e');

    await lifecycle.handleSessionExpired();

    expect(close).toHaveBeenCalledOnce();
    expect(reset).toHaveBeenCalledTimes(2);
    expect(open).toHaveBeenCalledOnce();
    expect(lifecycle.current()).toBeUndefined();
    expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'session-expired' });
  });

  it('serializes concurrent switches and only opens the latest requested subject', async () => {
    let finishClose: (() => void) | undefined;
    const closing = new Promise<void>((resolve) => {
      finishClose = resolve;
    });
    const events: string[] = [];
    let firstOpen = true;
    const lifecycle = new ScopedPersistenceLifecycle({
      namespace: 'example',
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest,
      open: ({ subject }) => {
        events.push(`open:${subject}`);
        const waitForClose = firstOpen;
        firstOpen = false;
        return Promise.resolve({
          close: async () => {
            events.push(`close:start:${subject}`);
            if (waitForClose) await closing;
            events.push(`close:end:${subject}`);
          },
        });
      },
      resetUserScopedState: () => {
        events.push('reset');
      },
    });
    await lifecycle.selectSubject('account-e');

    const selectF = lifecycle.selectSubject('account-f');
    await vi.waitFor(() => {
      expect(events).toContain('close:start:account-e');
    });
    const selectE = lifecycle.selectSubject('account-e');
    expect(lifecycle.current()).toBeUndefined();
    expect(lifecycle.state).toEqual({ status: 'initializing', subject: 'account-e' });
    expect(events).not.toContain('open:account-f');

    finishClose?.();
    await expect(selectF).resolves.toBeUndefined();
    await expect(selectE).resolves.toMatchObject({ subject: 'account-e' });
    expect(events).not.toContain('open:account-f');
    expect(events.indexOf('close:end:account-e')).toBeLessThan(
      events.lastIndexOf('open:account-e'),
    );
  });

  it('rechecks the exact server subject before adoption', async () => {
    const adopt = vi.fn(() => 'adopted');
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'account-e',
        readServerSubject: () => Promise.resolve('account-e'),
        adopt,
      }),
    ).resolves.toBe('adopted');
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'account-e',
        readServerSubject: () => Promise.resolve('account-f'),
        adopt,
      }),
    ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
    expect(adopt).toHaveBeenCalledOnce();
  });
});
