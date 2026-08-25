import { useEffect, useState } from 'react';
import { AccessibilityInfo, Platform } from 'react-native';

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

function mediaQuery(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(REDUCED_MOTION_QUERY)
    : null;
}

/** Tracks the reduced-motion preference and follows changes during the session. */
export function useReducedMotion(): boolean {
  const [reducedMotion, setReducedMotion] = useState(
    () => Platform.OS === 'web' && (mediaQuery()?.matches ?? false),
  );

  useEffect(() => {
    if (Platform.OS === 'web') {
      const query = mediaQuery();
      if (!query) return;
      const onChange = (event: MediaQueryListEvent) => {
        setReducedMotion(event.matches);
      };
      query.addEventListener('change', onChange);
      return () => {
        query.removeEventListener('change', onChange);
      };
    }

    let mounted = true;
    void AccessibilityInfo.isReduceMotionEnabled().then((enabled) => {
      if (mounted) setReducedMotion(enabled);
    });
    const subscription = AccessibilityInfo.addEventListener('reduceMotionChanged', (enabled) => {
      setReducedMotion(enabled);
    });
    return () => {
      mounted = false;
      subscription.remove();
    };
  }, []);

  return reducedMotion;
}
