import {
  createContext,
  createElement,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react';
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import * as SQLite from 'expo-sqlite';
import {
  PersistenceIdentityMismatchError,
  type OpenScopedPersistenceContext,
  type ScopedPersistenceLifecycleState,
  type ScopedPersistenceRegistryStore,
} from '@baukit/data-contracts';
import type { SessionExpiredReason } from '@baukit/auth-native';
import { ExpoSqliteStore } from '@baukit/data-contracts-expo-sqlite';

import type { Item } from './api';
import { loadAnalytics } from './analytics';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

import { PRODUCT_NAME } from './product';

const REGISTRY_KEY = `${PRODUCT_NAME}.local-data-registry.v1`;

class SecureStoreRegistry implements ScopedPersistenceRegistryStore {
  public read(): Promise<string | null> {
    return SecureStore.getItemAsync(REGISTRY_KEY);
  }

  public write(serialized: string): Promise<void> {
    return SecureStore.setItemAsync(REGISTRY_KEY, serialized);
  }
}

function digest(value: string): Promise<string> {
  return Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, value);
}

const registry = new SecureStoreRegistry();

function erasePartition({ storeName }: OpenScopedPersistenceContext): Promise<void> {
  return SQLite.deleteDatabaseAsync(`${storeName}.db`);
}

const localDataLifecycle = createProductPersistenceLifecycle<ExpoSqliteStore<Item>>({
  registry,
  digest,
  erase: erasePartition,
  open: async ({ storeName }) => {
    const database = await SQLite.openDatabaseAsync(`${storeName}.db`);
    const store = new ExpoSqliteStore<Item>(database, 'product', {
      closeDatabase: true,
    });
    try {
      await store.initialize();
      return store;
    } catch (cause) {
      await store.close().catch(() => undefined);
      throw cause;
    }
  },
  resetUserScopedState: async () => {
    const analytics = await loadAnalytics();
    analytics.reset();
  },
});

export type LocalDataState = ScopedPersistenceLifecycleState<ExpoSqliteStore<Item>>;

export interface AuthenticatedLocalData {
  readonly state: LocalDataState;
  readonly erase: (profileId: string) => Promise<void>;
  readonly blockIdentityMismatch: (error: PersistenceIdentityMismatchError) => Promise<void>;
}

const AuthenticatedLocalDataContext = createContext<AuthenticatedLocalData | undefined>(undefined);

interface AuthenticatedLocalDataProviderProps extends PropsWithChildren {
  readonly profileId: string | undefined;
  readonly sessionExpiredReason: SessionExpiredReason | undefined;
}

export function AuthenticatedLocalDataProvider({
  children,
  profileId,
  sessionExpiredReason,
}: AuthenticatedLocalDataProviderProps) {
  const localData = useAuthenticatedLocalDataState(profileId, sessionExpiredReason);
  return createElement(AuthenticatedLocalDataContext.Provider, { value: localData }, children);
}

export function useAuthenticatedLocalData(): AuthenticatedLocalData {
  const localData = useContext(AuthenticatedLocalDataContext);
  if (localData === undefined) {
    throw new Error(
      'useAuthenticatedLocalData must be used within AuthenticatedLocalDataProvider.',
    );
  }
  return localData;
}

/** Retain is the generated default; see docs/local-data-retention.md. */
function useAuthenticatedLocalDataState(
  profileId: string | undefined,
  sessionExpiredReason: SessionExpiredReason | undefined,
): AuthenticatedLocalData {
  const [state, setState] = useState<LocalDataState>(localDataLifecycle.state);
  const selectedProfileId = sessionExpiredReason === undefined ? profileId : undefined;

  useEffect(() => {
    let active = true;
    const transition = sessionExpiredReason !== undefined
      ? sessionExpiredReason === 'profile_erased'
        ? localDataLifecycle.handleProfileErased()
        : localDataLifecycle.handleSessionExpired()
      : selectedProfileId === undefined
        ? localDataLifecycle.clear()
        : localDataLifecycle.selectSubject(selectedProfileId);
    const updateState = () => {
      if (active) setState(localDataLifecycle.state);
    };
    void Promise.resolve().then(updateState);
    void transition.then(updateState, updateState);
    return () => {
      active = false;
    };
  }, [selectedProfileId, sessionExpiredReason]);

  const blockIdentityMismatch = useCallback(
    async (error: PersistenceIdentityMismatchError): Promise<void> => {
      await localDataLifecycle.blockIdentityMismatch(error);
      setState(localDataLifecycle.state);
    },
    [],
  );

  const erase = useCallback(async (profileId: string): Promise<void> => {
    if (localDataLifecycle.current()?.subject !== profileId) {
      throw new PersistenceIdentityMismatchError();
    }
    if (!(await localDataLifecycle.eraseActivePartition())) {
      throw new Error('No active local account partition to erase.');
    }
    setState(localDataLifecycle.state);
  }, []);

  const visibleState: LocalDataState = state.status === 'ready' &&
    (state.partition.subject !== profileId || sessionExpiredReason !== undefined)
      ? { status: 'signed-out' } : state;
  return { state: visibleState, erase, blockIdentityMismatch };
}
