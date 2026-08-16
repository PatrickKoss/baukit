import { AccessibilityInfo, Platform, findNodeHandle } from 'react-native';

export interface NativeBackgroundAccessibilityProps {
  readonly accessibilityElementsHidden?: boolean;
  readonly importantForAccessibility?: 'auto' | 'no-hide-descendants';
}

export interface NativeOverlayAccessibilityProps {
  readonly accessibilityViewIsModal?: boolean;
}

export interface NativeFocusRef {
  readonly current: object | null;
}

export interface NativeFocusDependencies {
  readonly findNodeHandle: (target: object) => number | null;
  readonly setAccessibilityFocus: (reactTag: number) => void;
}

const nativeFocusDependencies: NativeFocusDependencies = {
  findNodeHandle: (target) =>
    findNodeHandle(target as Parameters<typeof findNodeHandle>[0]),
  setAccessibilityFocus: (reactTag) => {
    AccessibilityInfo.setAccessibilityFocus(reactTag);
  },
};

export function backgroundAccessibilityProps(
  overlayVisible: boolean,
): NativeBackgroundAccessibilityProps {
  if (Platform.OS === 'ios') {
    return { accessibilityElementsHidden: overlayVisible };
  }

  return {
    importantForAccessibility: overlayVisible ? 'no-hide-descendants' : 'auto',
  };
}

export function overlayAccessibilityProps(): NativeOverlayAccessibilityProps {
  return Platform.OS === 'ios' ? { accessibilityViewIsModal: true } : {};
}

export function announceForAccessibility(message: string): void {
  if (message.trim().length > 0) {
    AccessibilityInfo.announceForAccessibility(message);
  }
}

export function isReduceMotionEnabled(): Promise<boolean> {
  return AccessibilityInfo.isReduceMotionEnabled();
}

export function focusAccessibilityTarget(
  targetRef: NativeFocusRef,
  dependencies: NativeFocusDependencies = nativeFocusDependencies,
): boolean {
  const target = targetRef.current;
  if (target === null) {
    return false;
  }

  const reactTag = dependencies.findNodeHandle(target);
  if (reactTag === null) {
    return false;
  }

  dependencies.setAccessibilityFocus(reactTag);
  return true;
}

export function focusAccessibilityTargetOnLayout(
  targetRef: NativeFocusRef,
  dependencies: NativeFocusDependencies = nativeFocusDependencies,
): () => void {
  return () => {
    focusAccessibilityTarget(targetRef, dependencies);
  };
}
