// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'web' as string };
const announceForAccessibility = vi.fn<(message: string) => void>();

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
  AccessibilityInfo: {
    announceForAccessibility: (message: string) => {
      announceForAccessibility(message);
    },
  },
}));

import { announce, DEFAULT_LIVE_REGION_ID } from './announce.js';

function region(id: string = DEFAULT_LIVE_REGION_ID): HTMLElement | null {
  return document.getElementById(id);
}

afterEach(() => {
  announceForAccessibility.mockReset();
  platform.OS = 'web';
  document.body.innerHTML = '';
});

describe('announce on native', () => {
  it.each(['ios', 'android'])('uses the platform accessibility API on %s', (os) => {
    platform.OS = os;

    announce('  Set saved  ', { assertive: true });

    expect(announceForAccessibility).toHaveBeenCalledWith('Set saved');
    expect(region()).toBeNull();
  });

  it('drops a blank message', () => {
    platform.OS = 'ios';

    announce('   ');

    expect(announceForAccessibility).not.toHaveBeenCalled();
  });
});

describe('announce on React Native Web', () => {
  it('writes into the live region instead of the native API', () => {
    announce('  Import complete  ', { assertive: true, liveRegionId: 'product-announcer' });

    expect(announceForAccessibility).not.toHaveBeenCalled();
    expect(region('product-announcer')?.textContent).toBe('Import complete');
    expect(region('product-announcer')?.getAttribute('aria-live')).toBe('assertive');
  });
});
