import { useCallback, useRef } from 'react';

export type SingleFlightRunner = <T>(operation: () => Promise<T>) => Promise<T | undefined>;

/**
 * A synchronous mutex for async UI mutations. React state alone cannot lock the
 * same tick, so a double tap would submit twice. A rejected call returns undefined.
 */
export function useSingleFlight(): SingleFlightRunner {
  const active = useRef(false);

  return useCallback(async <T>(operation: () => Promise<T>) => {
    if (active.current) return undefined;

    active.current = true;
    try {
      return await operation();
    } finally {
      active.current = false;
    }
  }, []);
}
