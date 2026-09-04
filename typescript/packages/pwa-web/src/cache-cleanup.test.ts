import { describe, expect, it, vi } from 'vitest';

import { cleanupCaches } from './cache-cleanup.js';

describe('cleanupCaches', () => {
  it('removes old cache versions without touching the current or unrelated caches', async () => {
    const deleted: string[] = [];

    await expect(
      cleanupCaches({
        ports: {
          listCacheNames: () =>
            Promise.resolve(['notes-app-v1', 'notes-app-v2', 'notes-media', 'another-product-v1']),
          deleteCache: (name) => {
            deleted.push(name);
            return Promise.resolve(true);
          },
        },
        shouldDelete: (name) => name.startsWith('notes-app-') && name !== 'notes-app-v2',
      }),
    ).resolves.toEqual({ matchedCount: 1, deletedCount: 1 });

    expect(deleted).toEqual(['notes-app-v1']);
  });

  it('removes only the previous identity partition after an account switch', async () => {
    const deleted: string[] = [];
    const previousPartition = 'account-17';

    await cleanupCaches({
      ports: {
        listCacheNames: () =>
          Promise.resolve([
            `notes-private-${previousPartition}`,
            'notes-private-account-42',
            'notes-static-v3',
          ]),
        deleteCache: (name) => {
          deleted.push(name);
          return Promise.resolve(true);
        },
      },
      shouldDelete: (name) => name === `notes-private-${previousPartition}`,
    });

    expect(deleted).toEqual(['notes-private-account-17']);
  });

  it('counts a cache that disappeared during cleanup as matched but not deleted', async () => {
    await expect(
      cleanupCaches({
        ports: {
          listCacheNames: () => Promise.resolve(['notes-app-v1']),
          deleteCache: () => Promise.resolve(false),
        },
        shouldDelete: () => true,
      }),
    ).resolves.toEqual({ matchedCount: 1, deletedCount: 0 });
  });

  it('starts all matching deletes before waiting for completion', async () => {
    const started: string[] = [];
    const finish: (() => void)[] = [];
    const cleanup = cleanupCaches({
      ports: {
        listCacheNames: () => Promise.resolve(['notes-app-v1', 'notes-app-v2']),
        deleteCache: (name) => {
          started.push(name);
          return new Promise((resolve) => {
            finish.push(() => {
              resolve(true);
            });
          });
        },
      },
      shouldDelete: () => true,
    });

    await vi.waitFor(() => {
      expect(started).toEqual(['notes-app-v1', 'notes-app-v2']);
    });
    for (const resolve of finish) {
      resolve();
    }
    await expect(cleanup).resolves.toEqual({ matchedCount: 2, deletedCount: 2 });
  });

  it('does not start deletion when cache selection fails', async () => {
    const deleteCache = vi.fn<(name: string) => Promise<boolean>>();

    await expect(
      cleanupCaches({
        ports: {
          listCacheNames: () => Promise.resolve(['notes-app-v1']),
          deleteCache,
        },
        shouldDelete: () => {
          throw new Error('invalid product cache policy');
        },
      }),
    ).rejects.toThrow('invalid product cache policy');
    expect(deleteCache).not.toHaveBeenCalled();
  });

  it('preserves cache deletion failures', async () => {
    const failure = new Error('cache storage unavailable');

    await expect(
      cleanupCaches({
        ports: {
          listCacheNames: () => Promise.resolve(['notes-app-v1']),
          deleteCache: () => Promise.reject(failure),
        },
        shouldDelete: () => true,
      }),
    ).rejects.toBe(failure);
  });
});
