import { AccessibilityInfo, Platform } from 'react-native';

import {
  announceForAccessibility,
  backgroundAccessibilityProps,
  focusAccessibilityTarget,
  focusAccessibilityTargetOnLayout,
  isReduceMotionEnabled,
  overlayAccessibilityProps,
  type NativeFocusDependencies,
} from './accessibility';

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

describe('native overlay accessibility props', () => {
  it('marks the iOS overlay as modal and hides only its background content', () => {
    usePlatform('ios');

    expect(overlayAccessibilityProps()).toEqual({
      accessibilityViewIsModal: true,
    });
    expect(backgroundAccessibilityProps(true)).toEqual({
      accessibilityElementsHidden: true,
    });
    expect(backgroundAccessibilityProps(false)).toEqual({
      accessibilityElementsHidden: false,
    });
  });

  it('removes Android background descendants while an overlay is visible', () => {
    usePlatform('android');

    expect(overlayAccessibilityProps()).toEqual({});
    expect(backgroundAccessibilityProps(true)).toEqual({
      importantForAccessibility: 'no-hide-descendants',
    });
    expect(backgroundAccessibilityProps(false)).toEqual({
      importantForAccessibility: 'auto',
    });
  });
});

describe('native accessibility events', () => {
  it.each(['ios', 'android'] as const)(
    'announces meaningful updates on %s',
    (platform) => {
      usePlatform(platform);
      const announce = jest
        .spyOn(AccessibilityInfo, 'announceForAccessibility')
        .mockImplementation();

      announceForAccessibility('Saved');
      announceForAccessibility('   ');

      expect(announce).toHaveBeenCalledTimes(1);
      expect(announce).toHaveBeenCalledWith('Saved');
    },
  );

  it.each(['ios', 'android'] as const)(
    'reads the reduced-motion preference on %s',
    async (platform) => {
      usePlatform(platform);
      const readPreference = jest
        .spyOn(AccessibilityInfo, 'isReduceMotionEnabled')
        .mockResolvedValue(true);

      await expect(isReduceMotionEnabled()).resolves.toBe(true);
      expect(readPreference).toHaveBeenCalledTimes(1);
    },
  );

  it.each(['ios', 'android'] as const)(
    'requests overlay focus on %s only after the layout callback',
    (platform) => {
      usePlatform(platform);
      const target = {};
      const dependencies: NativeFocusDependencies = {
        findNodeHandle: jest.fn(() => 41),
        setAccessibilityFocus: jest.fn(),
      };
      const onLayout = focusAccessibilityTargetOnLayout(
        { current: target },
        dependencies,
      );

      expect(dependencies.findNodeHandle).not.toHaveBeenCalled();
      expect(dependencies.setAccessibilityFocus).not.toHaveBeenCalled();

      onLayout();

      expect(dependencies.findNodeHandle).toHaveBeenCalledWith(target);
      expect(dependencies.setAccessibilityFocus).toHaveBeenCalledWith(41);
    },
  );

  it('does not request focus when the supplied stable ref is unavailable', () => {
    const dependencies: NativeFocusDependencies = {
      findNodeHandle: jest.fn(() => 41),
      setAccessibilityFocus: jest.fn(),
    };

    expect(focusAccessibilityTarget({ current: null }, dependencies)).toBe(
      false,
    );
    expect(dependencies.findNodeHandle).not.toHaveBeenCalled();
    expect(dependencies.setAccessibilityFocus).not.toHaveBeenCalled();
  });
});
