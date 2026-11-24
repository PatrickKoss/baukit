import Constants from 'expo-constants';

import { appScheme } from './app-scheme';
import { PRODUCT_NAME } from './product';

jest.mock('expo-constants', () => ({ __esModule: true, default: { expoConfig: {} } }));

function withScheme(scheme: unknown): string {
  Object.assign(Constants, { expoConfig: { scheme } });
  return appScheme();
}

describe('app scheme', () => {
  it('uses the scheme from the Expo config', () => {
    expect(withScheme(`${PRODUCT_NAME}-dev`)).toBe(`${PRODUCT_NAME}-dev`);
    expect(withScheme([`${PRODUCT_NAME}-dev`, PRODUCT_NAME])).toBe(`${PRODUCT_NAME}-dev`);
  });

  it.each([undefined, '', []])('falls back to the product name for %p', (scheme) => {
    expect(withScheme(scheme)).toBe(PRODUCT_NAME);
  });
});
