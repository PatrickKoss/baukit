import { useCallback, useEffect, useMemo, useState } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import {
  PersistenceIdentityMismatchError,
  type ScopedPersistenceLifecycleState,
  type ScopedPersistenceRegistryStore,
} from '@baukit/data-contracts';
import type { SessionExpiredReason } from '@baukit/auth-web';

import { analytics } from './analytics';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';
import { PRODUCT_NAME } from './product';

const REGISTRY_KEY = `${PRODUCT_NAME}:local-data-registry:v1`;

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
  readonly erase: (profileId: string) => Promise<void>;
  readonly blockIdentityMismatch: (
    error: PersistenceIdentityMismatchError,
  ) => Promise<void>;
}

/** Retain is the generated default; add Dexie opening inside `open` when persistence is added. */
export function useAuthenticatedLocalData(
  profileId: string | undefined,
  sessionExpiredReason: SessionExpiredReason | undefined,
  queryClient: QueryClient,
): AuthenticatedLocalData {
  const lifecycle = useMemo(
    () =>
      createProductPersistenceLifecycle<CachePartition>({
        registry: new LocalStorageRegistry(),
        open: () => Promise.resolve({ close: () => Promise.resolve() }),
        erase: () => {
          queryClient.clear();
          return Promise.resolve();
        },
        resetUserScopedState: () => {
          queryClient.clear();
          analytics.reset();
        },
      }),
    [queryClient],
  );
  const [state, setState] = useState<LocalDataState>(lifecycle.state);
  const selectedProfileId = sessionExpiredReason === undefined ? profileId : undefined;

  useEffect(() => {
    let active = true;
    const transition = sessionExpiredReason !== undefined
      ? sessionExpiredReason === 'profile_erased'
        ? lifecycle.handleProfileErased()
        : lifecycle.handleSessionExpired()
      : selectedProfileId === undefined
        ? lifecycle.clear()
        : lifecycle.selectSubject(selectedProfileId);
    const updateState = () => {
      if (active) setState(lifecycle.state);
    };
    void Promise.resolve().then(updateState);
    void transition.then(updateState, updateState);
    return () => {
      active = false;
    };
  }, [lifecycle, selectedProfileId, sessionExpiredReason]);

  const clear = useCallback(async (): Promise<void> => {
    await lifecycle.clear();
    setState(lifecycle.state);
  }, [lifecycle]);

  const erase = useCallback(async (profileId: string): Promise<void> => {
    if (lifecycle.current()?.subject !== profileId) {
      throw new PersistenceIdentityMismatchError();
    }
    if (!(await lifecycle.eraseActivePartition())) {
      throw new Error('No active local account partition to erase.');
    }
    setState(lifecycle.state);
  }, [lifecycle]);

  const blockIdentityMismatch = useCallback(
    async (error: PersistenceIdentityMismatchError): Promise<void> => {
      await lifecycle.blockIdentityMismatch(error);
      setState(lifecycle.state);
    },
    [lifecycle],
  );

  return { state, clear, erase, blockIdentityMismatch };
}
