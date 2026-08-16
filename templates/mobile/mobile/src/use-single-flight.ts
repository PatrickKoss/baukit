import { useState } from 'react';

export type SingleFlightRunner = <T>(operation: () => Promise<T>) => Promise<T | undefined>;

export function createSingleFlight(): SingleFlightRunner {
  let active = false;

  return async <T>(operation: () => Promise<T>) => {
    if (active) {
      return undefined;
    }

    active = true;
    try {
      return await operation();
    } finally {
      active = false;
    }
  };
}

/** A synchronous mutex for async UI mutations. React state alone cannot lock the same tick. */
export function useSingleFlight(): SingleFlightRunner {
  const [runner] = useState<SingleFlightRunner>(() => createSingleFlight());
  return runner;
}
