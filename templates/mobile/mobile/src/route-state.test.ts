import { describe, expect, it } from '@jest/globals';

import { deriveDetailRouteState } from './route-state';

const isItemId = (id: string) => /^[a-z]+-[0-9]+$/.test(id);

describe('deriveDetailRouteState', () => {
  it('rejects an invalid deep-link identifier before loading', () => {
    expect(
      deriveDetailRouteState({
        id: '../not-an-item',
        isValidId: isItemId,
        loading: true,
        error: new Error('must not mask invalid input'),
        value: undefined,
      }),
    ).toEqual({ status: 'invalid' });
  });

  it('returns loading, not-found, error, and ready without overlap', () => {
    expect(
      deriveDetailRouteState({
        id: 'item-1',
        isValidId: isItemId,
        loading: true,
        error: null,
        value: undefined,
      }),
    ).toEqual({ status: 'loading' });
    expect(
      deriveDetailRouteState({
        id: 'item-1',
        isValidId: isItemId,
        loading: false,
        error: null,
        value: undefined,
      }),
    ).toEqual({ status: 'not-found' });
    expect(
      deriveDetailRouteState({
        id: 'item-1',
        isValidId: isItemId,
        loading: false,
        error: new Error('offline'),
        value: undefined,
      }),
    ).toEqual({ status: 'error', message: 'offline' });
    expect(
      deriveDetailRouteState({
        id: 'item-1',
        isValidId: isItemId,
        loading: true,
        error: new Error('stale error'),
        value: { id: 'item-1' },
      }),
    ).toEqual({ status: 'ready', value: { id: 'item-1' } });
  });
});
