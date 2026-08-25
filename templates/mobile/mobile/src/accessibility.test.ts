import { announce } from '@baukit/a11y-core';
import { AccessibilityInfo, Platform } from 'react-native';

const originalPlatform = Platform.OS;

function usePlatform(platform: 'ios' | 'android'): void {
  Object.defineProperty(Platform, 'OS', {
    configurable: true,
    value: platform,
  });
}

afterEach(() => {
  Object.defineProperty(Platform, 'OS', {
    configurable: true,
    value: originalPlatform,
  });
  jest.restoreAllMocks();
});

describe('announcements through @baukit/a11y-core', () => {
  it.each(['ios', 'android'] as const)(
    'speaks a meaningful update on %s and drops a blank one',
    (platform) => {
      usePlatform(platform);
      const spoken = jest
        .spyOn(AccessibilityInfo, 'announceForAccessibility')
        .mockImplementation();

      announce('Saved');
      announce('   ');

      expect(spoken).toHaveBeenCalledTimes(1);
      expect(spoken).toHaveBeenCalledWith('Saved');
    },
  );
});
