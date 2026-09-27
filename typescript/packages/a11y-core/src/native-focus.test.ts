import type { RefObject } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'ios' as string };
const setAccessibilityFocus = vi.fn<(tag: number) => void>();
const findNodeHandle = vi.fn<(target: object) => number | null>();

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
  AccessibilityInfo: {
    setAccessibilityFocus: (tag: number) => {
      setAccessibilityFocus(tag);
    },
  },
  findNodeHandle: (target: object) => findNodeHandle(target),
}));

import { focusAccessibilityElement } from './native-focus.js';

const HEADING_TAG = 42;

function ref(current: object | null): RefObject<object | null> {
  return { current };
}

afterEach(() => {
  setAccessibilityFocus.mockReset();
  findNodeHandle.mockReset();
  platform.OS = 'ios';
});

describe('focusAccessibilityElement', () => {
  it('moves native screen-reader focus to the view tag', () => {
    const heading = { id: 'heading' };
    findNodeHandle.mockReturnValue(HEADING_TAG);

    expect(focusAccessibilityElement(ref(heading))).toBe(true);
    expect(findNodeHandle).toHaveBeenCalledWith(heading);
    expect(setAccessibilityFocus).toHaveBeenCalledWith(HEADING_TAG);
  });

  it('reports false when the native view is missing or unmounted', () => {
    expect(focusAccessibilityElement(ref(null))).toBe(false);
    expect(focusAccessibilityElement(undefined)).toBe(false);

    findNodeHandle.mockReturnValue(null);
    expect(focusAccessibilityElement(ref({}))).toBe(false);

    findNodeHandle.mockImplementation(() => {
      throw new Error('unmounted');
    });
    expect(focusAccessibilityElement(ref({}))).toBe(false);
    expect(setAccessibilityFocus).not.toHaveBeenCalled();
  });

  it('focuses the DOM element without scrolling on web', () => {
    platform.OS = 'web';
    const focus = vi.fn();

    expect(focusAccessibilityElement(ref({ focus }))).toBe(true);
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    expect(findNodeHandle).not.toHaveBeenCalled();
  });

  it('reports false on web when the element cannot take focus', () => {
    platform.OS = 'web';

    expect(focusAccessibilityElement(ref({}))).toBe(false);
    expect(focusAccessibilityElement(ref(null))).toBe(false);
  });
});
