import type { RouteFocusController, RouteFocusTarget } from '@baukit/a11y-core';
import type { RefObject } from 'react';

import { createRouteHeadingFocusEffect } from './route-heading-focus';

describe('Expo Router route heading focus adapter', () => {
  it('passes the mounted heading to the controller and returns its cleanup', () => {
    const heading = { focus: jest.fn() } as unknown as RouteFocusTarget;
    const headingRef = {
      current: heading,
    } as RefObject<RouteFocusTarget | null>;
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

  it('does not enter the route before its heading is ready', () => {
    const controller = {
      enterRoute: jest.fn(),
      dispose: jest.fn(),
    } satisfies RouteFocusController;
    const headingRef = { current: null } as RefObject<RouteFocusTarget | null>;

    expect(createRouteHeadingFocusEffect(controller, headingRef, false)).toBeUndefined();
    expect(controller.enterRoute).not.toHaveBeenCalled();
  });

  it('does nothing on native where the DOM controller is unavailable', () => {
    const headingRef = { current: null } as RefObject<RouteFocusTarget | null>;

    expect(createRouteHeadingFocusEffect(null, headingRef, true)).toBeUndefined();
  });
});
