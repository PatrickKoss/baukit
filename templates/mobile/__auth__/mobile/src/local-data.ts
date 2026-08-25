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
  type ScopedPersistenceLifecycleState,
  type ScopedPersistenceRegistryStore,
} from '@baukit/data-contracts';
import { ExpoSqliteStore } from '@baukit/data-contracts-expo-sqlite';

import type { Item } from './api';
import { loadAnalytics } from './analytics';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

const REGISTRY_KEY = '{{ context.app_name }}:local-data-registry:v1';

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

const localDataLifecycle = createProductPersistenceLifecycle<ExpoSqliteStore<Item>>({
  registry: new SecureStoreRegistry(),
  digest,
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
  readonly blockIdentityMismatch: (error: PersistenceIdentityMismatchError) => Promise<void>;
}

const AuthenticatedLocalDataContext = createContext<AuthenticatedLocalData | undefined>(undefined);

interface AuthenticatedLocalDataProviderProps extends PropsWithChildren {
  readonly subject: string | undefined;
  readonly sessionExpired: boolean;
}

export function AuthenticatedLocalDataProvider({
  children,
  subject,
  sessionExpired,
}: AuthenticatedLocalDataProviderProps) {
  const localData = useAuthenticatedLocalDataState(subject, sessionExpired);
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
  subject: string | undefined,
  sessionExpired: boolean,
): AuthenticatedLocalData {
  const [state, setState] = useState<LocalDataState>(localDataLifecycle.state);

  useEffect(() => {
    let active = true;
    const transition = sessionExpired
      ? localDataLifecycle.handleSessionExpired()
      : subject === undefined
        ? localDataLifecycle.clear()
        : localDataLifecycle.selectSubject(subject).then(() => undefined);
    void Promise.resolve().then(() => {
      if (active) setState(localDataLifecycle.state);
    });
    void transition.then(
      () => {
        if (active) setState(localDataLifecycle.state);
      },
      () => {
        if (active) setState(localDataLifecycle.state);
      },
    );
    return () => {
      active = false;
    };
  }, [sessionExpired, subject]);

  const blockIdentityMismatch = useCallback(
    async (error: PersistenceIdentityMismatchError): Promise<void> => {
      await localDataLifecycle.blockIdentityMismatch(error);
      setState(localDataLifecycle.state);
    },
    [],
  );

  return { state, blockIdentityMismatch };
}
