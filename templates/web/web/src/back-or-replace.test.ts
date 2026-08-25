import { describe, expect, it, vi } from 'vitest';

import { backOrReplace, type BackOrReplaceNavigation } from './back-or-replace';

function navigation(canGoBack: boolean): BackOrReplaceNavigation {
  return {
    canGoBack: () => canGoBack,
    back: vi.fn(),
    replace: vi.fn(),
  };
}

describe('backOrReplace', () => {
  it('uses available history', () => {
    const history = navigation(true);
    backOrReplace(history, '/items');

    expect(history.back).toHaveBeenCalledOnce();
    expect(history.replace).not.toHaveBeenCalled();
  });

  it('replaces a direct load with the semantic destination', () => {
    const directLoad = navigation(false);
    backOrReplace(directLoad, '/items');

    expect(directLoad.back).not.toHaveBeenCalled();
    expect(directLoad.replace).toHaveBeenCalledWith('/items');
  });
});
