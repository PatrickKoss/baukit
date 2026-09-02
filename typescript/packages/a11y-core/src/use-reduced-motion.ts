import { useEffect, useState } from 'react';
import { AccessibilityInfo, Platform } from 'react-native';

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

export interface ReducedMotionPreference {
  reducedMotion: boolean;
  resolved: boolean;
}

function mediaQuery(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(REDUCED_MOTION_QUERY)
    : null;
}

/** Tracks when the reduced-motion preference is ready and follows later changes. */
export function useReducedMotionPreference(): ReducedMotionPreference {
  const [preference, setPreference] = useState<ReducedMotionPreference>(() => {
    const web = Platform.OS === 'web';
    return {
      reducedMotion: web && (mediaQuery()?.matches ?? false),
      resolved: web,
    };
  });

  useEffect(() => {
    if (Platform.OS === 'web') {
      const query = mediaQuery();
      if (!query) return;
      const onChange = (event: MediaQueryListEvent) => {
        setPreference({ reducedMotion: event.matches, resolved: true });
      };
      query.addEventListener('change', onChange);
      return () => {
        query.removeEventListener('change', onChange);
      };
    }

    let mounted = true;
    let preferenceChanged = false;
    const settle = (reducedMotion?: boolean) => {
      if (!mounted) return;
      setPreference((current) => ({
        reducedMotion:
          reducedMotion === undefined || preferenceChanged ? current.reducedMotion : reducedMotion,
        resolved: true,
      }));
    };
    void AccessibilityInfo.isReduceMotionEnabled().then(settle, () => {
      settle();
    });
    const subscription = AccessibilityInfo.addEventListener('reduceMotionChanged', (enabled) => {
      if (!mounted) return;
      preferenceChanged = true;
      setPreference({ reducedMotion: enabled, resolved: true });
    });
    return () => {
      mounted = false;
      subscription.remove();
    };
  }, []);

  return preference;
}

/** Tracks the reduced-motion preference and follows changes during the session. */
export function useReducedMotion(): boolean {
  return useReducedMotionPreference().reducedMotion;
}
