import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import {
  PersistenceIdentityMismatchError,
  removeScopedPersistenceRegistryEntry,
  type OpenScopedPersistenceContext,
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
  const registry = useMemo(() => new LocalStorageRegistry(), []);
  const erasePartition: (partition: OpenScopedPersistenceContext) => Promise<void> = useCallback(() => {
    queryClient.clear();
    return Promise.resolve();
  }, [queryClient]);
  const lifecycle = useMemo(
    () =>
      createProductPersistenceLifecycle<CachePartition>({
        registry,
        open: () => Promise.resolve({ close: () => Promise.resolve() }),
        erase: erasePartition,
        resetUserScopedState: () => {
          queryClient.clear();
          analytics.reset();
        },
      }),
    [erasePartition, queryClient, registry],
  );
  const [state, setState] = useState<LocalDataState>(lifecycle.state);
  const expiryTransition = useRef<{
    readonly reason: SessionExpiredReason;
    readonly transition: Promise<void>;
  } | undefined>(undefined);
  const lastPartition = useRef<OpenScopedPersistenceContext | undefined>(undefined);
  const expireSession = useCallback(async (reason: SessionExpiredReason): Promise<void> => {
    if (reason !== 'profile_erased') {
      await lifecycle.handleSessionExpired();
      return;
    }
    const previous = lastPartition.current;
    const erasure = lifecycle.eraseActivePartition();
    const blocked = lifecycle.handleSessionExpired();
    const [erased] = await Promise.all([erasure, blocked]);
    if (!erased && previous !== undefined) {
      // Token clearing can close the partition before the expiry event arrives.
      await erasePartition(previous);
      await removeScopedPersistenceRegistryEntry({
        namespace: PRODUCT_NAME, subject: previous.subject, registry,
      });
    }
    if (lastPartition.current === previous) lastPartition.current = undefined;
  }, [erasePartition, lifecycle, registry]);

  useEffect(() => {
    let active = true;
    if (profileId !== undefined && lastPartition.current?.subject !== profileId) {
      lastPartition.current = undefined;
    }
    if (sessionExpiredReason === undefined) expiryTransition.current = undefined;
    else if (expiryTransition.current?.reason !== sessionExpiredReason) {
      expiryTransition.current = {
        reason: sessionExpiredReason,
        transition: expireSession(sessionExpiredReason),
      };
    }
    const transition = expiryTransition.current?.transition ?? (
      profileId === undefined
        ? lifecycle.clear()
        : lifecycle.selectSubject(profileId).then((partition) => {
            if (partition !== undefined) lastPartition.current = {
              subject: partition.subject,
              storeName: partition.storeName,
            };
          })
    );
    void Promise.resolve().then(() => {
      if (active) setState(lifecycle.state);
    });
    void transition.then(
      () => {
        if (active) setState(lifecycle.state);
      },
      (cause: unknown) => {
        if (!active) return;
        setState(sessionExpiredReason !== 'profile_erased' && lifecycle.state.status === 'blocked' ? lifecycle.state : {
          status: 'blocked',
          reason: 'initialization-failed',
          error: cause instanceof Error ? cause : new Error('Local data cleanup failed.'),
        });
      },
    );
    return () => {
      active = false;
    };
  }, [expireSession, lifecycle, profileId, sessionExpiredReason]);

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
    lastPartition.current = undefined;
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
