import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  PersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from '@baukit/data-contracts';

import { apiSessionSubject, setApiSessionSubject } from './api-session';

afterEach(() => {
  setApiSessionSubject(undefined);
});

describe('sync adoption token fence', () => {
  it('refuses adoption when the API session belongs to another profile', async () => {
    setApiSessionSubject('profile-b');
    const adopt = vi.fn();
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'profile-a',
        tokenSubject: apiSessionSubject,
        readServerSubject: () => Promise.resolve('profile-a'),
        adopt,
      }),
    ).rejects.toBeInstanceOf(PersistenceIdentityMismatchError);
    expect(adopt).not.toHaveBeenCalled();
  });

  it('adopts when the API session matches the partition', async () => {
    setApiSessionSubject('profile-a');
    await expect(
      recheckServerSubjectBeforeSyncAdoption({
        partitionSubject: 'profile-a',
        tokenSubject: apiSessionSubject,
        readServerSubject: () => Promise.resolve('profile-a'),
        adopt: () => 'adopted',
      }),
    ).resolves.toBe('adopted');
  });
});
