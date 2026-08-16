import { useCallback, useEffect, useMemo, useState } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import {
  PersistenceIdentityMismatchError,
  type ScopedPersistenceLifecycleState,
  type ScopedPersistenceRegistryStore,
} from '@baukit/data-contracts';

import { analytics } from './analytics';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

const REGISTRY_KEY = '{{ context.app_name }}:local-data-registry:v1';

interface CachePartition {
  close(): Promise<void>;
}

class LocalStorageRegistry implements ScopedPersistenceRegistryStore {
  public read(): Promise<string | null> {
    return Promise.resolve(localStorage.getItem(REGISTRY_KEY));
  }

  public write(serialized: string): Promise<void> {
    localStorage.setItem(REGISTRY_KEY, serialized);
    return Promise.resolve();
  }
}

export type LocalDataState = ScopedPersistenceLifecycleState<CachePartition>;

export interface AuthenticatedLocalData {
  readonly state: LocalDataState;
  readonly clear: () => Promise<void>;
  readonly blockIdentityMismatch: (
    error: PersistenceIdentityMismatchError,
  ) => Promise<void>;
}

/** Retain is the generated default; add Dexie opening inside `open` when persistence is added. */
export function useAuthenticatedLocalData(
  subject: string | undefined,
  sessionExpired: boolean,
  queryClient: QueryClient,
): AuthenticatedLocalData {
  const lifecycle = useMemo(
    () =>
      createProductPersistenceLifecycle<CachePartition>({
        registry: new LocalStorageRegistry(),
        open: () => Promise.resolve({ close: () => Promise.resolve() }),
        resetUserScopedState: () => {
          queryClient.clear();
          analytics.reset();
        },
      }),
    [queryClient],
  );
  const [state, setState] = useState<LocalDataState>(lifecycle.state);

  useEffect(() => {
    let active = true;
    const transition = sessionExpired
      ? lifecycle.handleSessionExpired()
      : subject === undefined
        ? lifecycle.clear()
        : lifecycle.selectSubject(subject).then(() => undefined);
    void Promise.resolve().then(() => {
      if (active) setState(lifecycle.state);
    });
    void transition.then(
      () => {
        if (active) setState(lifecycle.state);
      },
      () => {
        if (active) setState(lifecycle.state);
      },
    );
    return () => {
      active = false;
    };
  }, [lifecycle, sessionExpired, subject]);

  const clear = useCallback(async (): Promise<void> => {
    await lifecycle.clear();
    setState(lifecycle.state);
  }, [lifecycle]);

  const blockIdentityMismatch = useCallback(
    async (error: PersistenceIdentityMismatchError): Promise<void> => {
      await lifecycle.blockIdentityMismatch(error);
      setState(lifecycle.state);
    },
    [lifecycle],
  );

  return { state, clear, blockIdentityMismatch };
}
