import { describe, expect, it, jest } from '@jest/globals';
import { useRouter } from 'expo-router';

import { backOrReplace, type BackOrReplaceNavigation, useBackOrReplace } from './back-or-replace';

jest.mock('expo-router', () => ({ useRouter: jest.fn() }));

function navigation(canGoBack?: boolean): BackOrReplaceNavigation {
  return {
    ...(canGoBack === undefined ? {} : { canGoBack: () => canGoBack }),
    back: jest.fn(),
    replace: jest.fn(),
  };
}

describe('backOrReplace', () => {
  it('uses available history', () => {
    const history = navigation(true);
    backOrReplace(history, '/items');

    expect(history.back).toHaveBeenCalledTimes(1);
    expect(history.replace).not.toHaveBeenCalled();
  });

  it('replaces a direct load with the semantic destination', () => {
    const directLoad = navigation(false);
    backOrReplace(directLoad, '/items');

    expect(directLoad.back).not.toHaveBeenCalled();
    expect(directLoad.replace).toHaveBeenCalledWith('/items');
  });

  it('replaces when history availability is unknown', () => {
    const unknownHistory = navigation();
    backOrReplace(unknownHistory, '/items');

    expect(unknownHistory.back).not.toHaveBeenCalled();
    expect(unknownHistory.replace).toHaveBeenCalledWith('/items');
  });

  it('adapts Expo Router', () => {
    const router = navigation(false);
    jest.mocked(useRouter).mockReturnValue(router as ReturnType<typeof useRouter>);

    useBackOrReplace()('/items');

    expect(router.replace).toHaveBeenCalledWith('/items');
  });
});
