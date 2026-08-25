import { describe, expect, it } from 'vitest';

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

  it('returns mutually exclusive terminal states', () => {
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
        loading: true,
        error: null,
        value: { id: 'item-1' },
      }),
    ).toEqual({ status: 'ready', value: { id: 'item-1' } });
  });
});
