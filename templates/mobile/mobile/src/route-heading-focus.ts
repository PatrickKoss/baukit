import {
  createRouteFocusController,
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

export function createRouteHeadingFocusEffect<T>(
  controller: RouteFocusController | null,
  headingRef: RefObject<T | null>,
  ready: boolean,
): (() => void) | undefined {
  if (!ready || controller === null) return undefined;
  return controller.enterRoute(() => headingRef.current as RouteFocusTarget | null);
}

export function useRouteHeadingFocus<T>(headingRef: RefObject<T | null>, ready = true): void {
  useFocusEffect(
    useCallback(
      () => createRouteHeadingFocusEffect(routeFocusController(), headingRef, ready),
      [headingRef, ready],
    ),
  );
}
