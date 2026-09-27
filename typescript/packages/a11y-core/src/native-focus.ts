import { AccessibilityInfo, findNodeHandle, Platform } from 'react-native';

import { asFocusTarget, hostElement, type HostRef } from './dom-boundary.js';

/** Native view tag behind a ref, or null when the ref is empty or unmounted. */
export function nativeNodeHandle(ref: HostRef | undefined): number | null {
  const host = hostElement(ref);
  if (host === null) return null;
  try {
    return findNodeHandle(host as Parameters<typeof findNodeHandle>[0]);
  } catch {
    return null;
  }
}

/**
 * Moves screen-reader focus to the element behind `ref`. Web focuses the DOM
 * element without scrolling; native asks VoiceOver or TalkBack to focus the
 * view. Returns false when there was nothing to focus.
 */
export function focusAccessibilityElement(ref: HostRef | undefined): boolean {
  if (Platform.OS === 'web') {
    const target = asFocusTarget(ref);
    target?.focus({ preventScroll: true });
    return target !== null;
  }
  const handle = nativeNodeHandle(ref);
  if (handle === null) return false;
  AccessibilityInfo.setAccessibilityFocus(handle);
  return true;
}
