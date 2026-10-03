import { describe, expect, it, vi } from 'vitest';

import deletionOutcomeFixtures from '../../../../fixtures/product-experience/deletion-outcomes.json' with { type: 'json' };

import {
  AmbiguousProductProfileErasureError,
  type ErasureReceipt,
  type ProductProfileErasureDependencies,
  type ProductProfileErasureResult,
  eraseProductProfile,
} from './erasure.js';

interface DeletionOutcomeFixture {
  readonly name: string;
  readonly serverRetry: 'not-required' | 'retry' | 'reconcile';
  readonly sessionRetained: boolean;
  readonly result: ProductProfileErasureResult;
}

const fixtures = deletionOutcomeFixtures as readonly DeletionOutcomeFixture[];

const receipt: ErasureReceipt = { operationId: 'erase-1', status: 'completed' };

function namedError(name: string): Error {
  const error = new Error('email=user@example.test token=secret request body resource content');
  error.name = name;
  return error;
}

function expected(name: string): ProductProfileErasureResult {
  const fixture = fixtures.find((candidate) => candidate.name === name);
  if (fixture === undefined) throw new Error(`Missing deletion outcome fixture: ${name}`);
  return fixture.result;
}

function dependencies(outcome: string, events: string[] = []): ProductProfileErasureDependencies {
  return {
    beforeServerErase:
      outcome === 'warnings-only'
        ? [
            () => {
              events.push('before');
              return Promise.reject(namedError('SecretHookError'));
            },
          ]
        : [],
    eraseServerProfile: () => {
      events.push('server');
      if (outcome === 'server-failure') return Promise.reject(namedError('TypeError'));
      if (outcome === 'ambiguous') {
        return Promise.reject(new AmbiguousProductProfileErasureError(namedError('TimeoutError')));
      }
      return Promise.resolve(receipt);
    },
    eraseLocalPartition: () => {
      events.push('local');
      return outcome === 'local-failure'
        ? Promise.reject(namedError('RangeError'))
        : Promise.resolve();
    },
    signOut: () => {
      events.push('sign-out');
      return outcome === 'signout-failure'
        ? Promise.reject(namedError('AbortError'))
        : Promise.resolve();
    },
  };
}

describe('product-profile erasure', () => {
  it.each(fixtures)('matches the shared $name outcome fixture', async (fixture) => {
    await expect(eraseProductProfile(dependencies(fixture.name))).resolves.toEqual(fixture.result);
  });

  it.each(fixtures)('encodes retry and session expectations for $name', (fixture) => {
    const status = fixture.result.status;
    expect(fixture.serverRetry).toBe(
      status === 'server-failure' ? 'retry' : status === 'ambiguous' ? 'reconcile' : 'not-required',
    );
    expect(fixture.sessionRetained).toBe(
      status === 'server-failure' || status === 'ambiguous' || status === 'signout-failure',
    );
  });

  it('continues after a failed pre-server hook and reports a safe warning', async () => {
    const events: string[] = [];
    const result = await eraseProductProfile(dependencies('warnings-only', events));

    expect(result).toEqual(expected('warnings-only'));
    expect(events).toEqual(['before', 'server', 'local', 'sign-out']);
    expect(JSON.stringify(result)).not.toMatch(
      /user@example|secret|request body|resource content/u,
    );
  });

  it.each(['server-failure', 'ambiguous'])(
    '%s preserves local data and the session',
    async (name) => {
      const events: string[] = [];

      await expect(eraseProductProfile(dependencies(name, events))).resolves.toEqual(
        expected(name),
      );
      expect(events).toEqual(['server']);
    },
  );

  it.each([42, '', '   '])(
    'treats a pending receipt with operation ID %j as an ambiguous response',
    async (operationId) => {
      const events: string[] = [];
      const malformedReceipt: ErasureReceipt = { operationId: 'erase-1', status: 'pending' };
      Reflect.set(malformedReceipt, 'operationId', operationId);

      await expect(
        eraseProductProfile({
          eraseServerProfile: () => {
            events.push('server');
            return Promise.resolve(malformedReceipt);
          },
          eraseLocalPartition: () => {
            events.push('local');
            return Promise.resolve();
          },
          signOut: () => {
            events.push('sign-out');
            return Promise.resolve();
          },
        }),
      ).resolves.toEqual({
        status: 'ambiguous',
        error: { stage: 'server', cause: 'TypeError' },
        warnings: [],
      });
      expect(events).toEqual(['server']);
    },
  );

  it.each(['erase-1', null])(
    'clears local data and signs out for pending operation %j',
    async (operationId) => {
      const events: string[] = [];
      const pending: ErasureReceipt = { operationId, status: 'pending' };
      const result = await eraseProductProfile({
        ...dependencies('success', events),
        eraseServerProfile: () => {
          events.push('server');
          return Promise.resolve(pending);
        },
      });

      expect(result).toEqual({ status: 'pending', receipt: pending, warnings: [] });
      expect(events).toEqual(['server', 'local', 'sign-out']);
    },
  );

  it.each(['local-failure', 'signout-failure'])(
    'keeps %s visible after a pending receipt',
    async (outcome) => {
      const events: string[] = [];
      const pending: ErasureReceipt = { operationId: 'erase-1', status: 'pending' };
      const result = await eraseProductProfile({
        ...dependencies(outcome, events),
        eraseServerProfile: () => {
          events.push('server');
          return Promise.resolve(pending);
        },
      });
      expect(result).toMatchObject({ status: outcome, receipt: pending });
      expect(events).toEqual(['server', 'local', 'sign-out']);
    },
  );

  it('signs out after local deletion fails and keeps both failures visible', async () => {
    const result = await eraseProductProfile({
      ...dependencies('local-failure'),
      signOut: () => Promise.reject(namedError('AbortError')),
    });

    expect(result).toEqual({
      ...expected('local-failure'),
      signOutError: { stage: 'sign-out', cause: 'AbortError' },
    });
  });

  it('is safe to invoke repeatedly against idempotent dependencies', async () => {
    const eraseServerProfile = vi.fn(() => Promise.resolve(receipt));
    const eraseLocalPartition = vi.fn(() => Promise.resolve());
    const signOut = vi.fn(() => Promise.resolve());
    const deps = { eraseServerProfile, eraseLocalPartition, signOut };

    await expect(eraseProductProfile(deps)).resolves.toEqual(expected('success'));
    await expect(eraseProductProfile(deps)).resolves.toEqual(expected('success'));
    expect(eraseServerProfile).toHaveBeenCalledTimes(2);
    expect(eraseLocalPartition).toHaveBeenCalledTimes(2);
    expect(signOut).toHaveBeenCalledTimes(2);
  });
});
