import {
  createRouteFocusController,
  focusAccessibilityElement,
  type RouteFocusController,
  type RouteFocusTarget,
} from '@baukit/a11y-core';
import { useFocusEffect } from 'expo-router';
import { useCallback, type RefObject } from 'react';
import { Platform } from 'react-native';

let sharedController: RouteFocusController | null | undefined;

function routeFocusController(): RouteFocusController | null {
  if (sharedController !== undefined) return sharedController;
  sharedController =
    Platform.OS === 'web' && typeof document !== 'undefined' ? createRouteFocusController() : null;
  return sharedController;
}

/**
 * Web hands the heading to the DOM route focus controller. Native has no
 * controller, so screen-reader focus moves to the heading one frame after the
 * route gains focus.
 */
export function createRouteHeadingFocusEffect(
  controller: RouteFocusController | null,
  headingRef: RefObject<object | null>,
  ready: boolean,
): (() => void) | undefined {
  if (!ready) return undefined;
  if (controller !== null) {
    return controller.enterRoute(() => headingRef.current as RouteFocusTarget | null);
  }
  const frame = requestAnimationFrame(() => {
    focusAccessibilityElement(headingRef);
  });
  return () => {
    cancelAnimationFrame(frame);
  };
}

export function useRouteHeadingFocus(headingRef: RefObject<object | null>, ready = true): void {
  useFocusEffect(
    useCallback(
      () => createRouteHeadingFocusEffect(routeFocusController(), headingRef, ready),
      [headingRef, ready],
    ),
  );
}
