import type { RouteFocusController, RouteFocusTarget } from '@baukit/a11y-core';
import type { RefObject } from 'react';

const mockFocusAccessibilityElement = jest.fn<undefined, [unknown]>();

jest.mock('@baukit/a11y-core', () => ({
  ...jest.requireActual<typeof import('@baukit/a11y-core')>('@baukit/a11y-core'),
  focusAccessibilityElement: (ref: unknown) => {
    mockFocusAccessibilityElement(ref);
    return true;
  },
}));

import { createRouteHeadingFocusEffect } from './route-heading-focus';

describe('Expo Router route heading focus adapter', () => {
  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('passes the mounted heading to the web controller and returns its cleanup', () => {
    const heading = { focus: jest.fn() } as unknown as RouteFocusTarget;
    const headingRef = { current: heading } as RefObject<object | null>;
    const cleanup = jest.fn();
    let target: (() => RouteFocusTarget | null) | undefined;
    const controller = {
      enterRoute: jest.fn((nextTarget: () => RouteFocusTarget | null) => {
        target = nextTarget;
        return cleanup;
      }),
      dispose: jest.fn(),
    } satisfies RouteFocusController;

    const effectCleanup = createRouteHeadingFocusEffect(controller, headingRef, true);

    expect(controller.enterRoute).toHaveBeenCalledTimes(1);
    expect(target?.()).toBe(heading);
    expect(effectCleanup).toBe(cleanup);
  });

  it('does nothing before the heading is ready', () => {
    const controller = { enterRoute: jest.fn(), dispose: jest.fn() } satisfies RouteFocusController;
    const requestFrame = jest.spyOn(globalThis, 'requestAnimationFrame');
    const headingRef = { current: null } as RefObject<object | null>;

    expect(createRouteHeadingFocusEffect(controller, headingRef, false)).toBeUndefined();
    expect(createRouteHeadingFocusEffect(null, headingRef, false)).toBeUndefined();
    expect(controller.enterRoute).not.toHaveBeenCalled();
    expect(requestFrame).not.toHaveBeenCalled();
  });

  it('moves native screen-reader focus to the heading on the next frame', () => {
    let frameCallback: FrameRequestCallback | undefined;
    jest.spyOn(globalThis, 'requestAnimationFrame').mockImplementation((callback) => {
      frameCallback = callback;
      return 7;
    });
    const headingRef = { current: {} } as RefObject<object | null>;

    createRouteHeadingFocusEffect(null, headingRef, true);
    expect(mockFocusAccessibilityElement).not.toHaveBeenCalled();

    frameCallback?.(0);
    expect(mockFocusAccessibilityElement).toHaveBeenCalledWith(headingRef);
  });

  it('cancels the pending native focus when the route loses focus', () => {
    jest.spyOn(globalThis, 'requestAnimationFrame').mockReturnValue(7);
    const cancelFrame = jest.spyOn(globalThis, 'cancelAnimationFrame').mockReturnValue(undefined);

    const cleanup = createRouteHeadingFocusEffect(null, { current: {} }, true);
    cleanup?.();

    expect(cancelFrame).toHaveBeenCalledWith(7);
  });
});
