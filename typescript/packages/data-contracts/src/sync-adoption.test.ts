import { describe, expect, it, vi } from 'vitest';

import {
  PersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from './identity.js';

describe('fenced sync adoption', () => {
  it('refuses to adopt when the lifecycle moved during the recheck', async () => {
    const lifecycle = { transitionRevision: 1 };
    const adopt = vi.fn();
    const adoption = recheckServerSubjectBeforeSyncAdoption({
      partitionSubject: 'account-a',
      lifecycle,
      readServerSubject: () => {
        lifecycle.transitionRevision = 2;
        return Promise.resolve('account-a');
      },
      adopt,
    });
    await expect(adoption).rejects.toBeInstanceOf(PersistenceIdentityMismatchError);
    expect(adopt).not.toHaveBeenCalled();
  });

  it('refuses when the token now belongs to another subject', async () => {
    const adopt = vi.fn();
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'account-a',
        tokenSubject: () => 'account-b',
        readServerSubject: () => Promise.resolve('account-a'),
        adopt,
      }),
    ).rejects.toBeInstanceOf(PersistenceIdentityMismatchError);
    expect(adopt).not.toHaveBeenCalled();
  });

  it('aborts the adopt signal once another tab replaces the token mid-push', async () => {
    let tokenSubject = 'account-a';
    const pushed: string[] = [];
    await recheckServerSubjectBeforeSyncAdoption({
      partitionSubject: 'account-a',
      tokenSubject: () => tokenSubject,
      readServerSubject: () => Promise.resolve('account-a'),
      adopt: (fence) => {
        for (const item of ['one', 'two']) {
          if (!fence.owned()) return;
          pushed.push(item);
          tokenSubject = 'account-b';
        }
      },
    });
    expect(pushed).toEqual(['one']);
    expect.hasAssertions();
  });

  it('honours the caller signal', async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'account-a',
        signal: controller.signal,
        readServerSubject: () => Promise.resolve('account-a'),
        adopt: vi.fn(),
      }),
    ).rejects.toThrow();
  });
});
