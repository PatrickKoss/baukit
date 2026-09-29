import { useMemo, useSyncExternalStore } from 'react';

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

export interface ReducedMotionPreference {
  reducedMotion: boolean;
  resolved: boolean;
}

/** The `prefers-reduced-motion` media query, or `null` where the host has no `matchMedia`. */
export function reducedMotionQuery(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(REDUCED_MOTION_QUERY)
    : null;
}

function subscribeToQuery(onChange: () => void): () => void {
  const query = reducedMotionQuery();
  if (query === null) {
    return () => undefined;
  }
  query.addEventListener('change', onChange);
  return () => {
    query.removeEventListener('change', onChange);
  };
}

function subscribeToNothing(): () => void {
  return () => undefined;
}

const readQuery = () => reducedMotionQuery()?.matches ?? false;
const onClient = () => true;
const onServer = () => false;

/**
 * Follows `prefers-reduced-motion` through `matchMedia`. `resolved` is `false` only while
 * rendering on a server, which cannot see the user's setting.
 */
export function useReducedMotionPreference(): ReducedMotionPreference {
  const reducedMotion = useSyncExternalStore(subscribeToQuery, readQuery, onServer);
  const resolved = useSyncExternalStore(subscribeToNothing, onClient, onServer);
  return useMemo(() => ({ reducedMotion, resolved }), [reducedMotion, resolved]);
}

/** Follows `prefers-reduced-motion` and its changes during the session. */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribeToQuery, readQuery, onServer);
}
