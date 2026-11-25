import {
  createContext,
  createElement,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactElement,
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
import { setApiSessionSubject } from './api-session';
import { loadAnalytics } from './analytics';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

import { PRODUCT_NAME } from './product';
import { profileErasureMarker } from './erasure-marker';

const REGISTRY_KEY = `${PRODUCT_NAME}.local-data-registry.v1`;

const REGISTRY_CHUNK_COUNT_KEY = `${REGISTRY_KEY}.chunks`;
/** SecureStore warns above 2048 bytes per item; 500 UTF-16 units stay below it for any text. */
const REGISTRY_CHUNK_LENGTH = 500;

/** Splits the registry across items so it can outgrow one SecureStore item. */
class SecureStoreRegistry implements ScopedPersistenceRegistryStore {
  public async read(): Promise<string | null> {
    const count = await SecureStore.getItemAsync(REGISTRY_CHUNK_COUNT_KEY);
    if (count === null) return SecureStore.getItemAsync(REGISTRY_KEY);
    const chunks: string[] = [];
    for (let index = 0; index < Number(count); index += 1) {
      const chunk = await SecureStore.getItemAsync(`${REGISTRY_KEY}.${String(index)}`);
      if (chunk === null) throw new PersistenceIdentityMismatchError();
      chunks.push(chunk);
    }
    return chunks.join('');
  }

  public async write(serialized: string): Promise<void> {
    const previous = Number((await SecureStore.getItemAsync(REGISTRY_CHUNK_COUNT_KEY)) ?? '0');
    const chunks = chunk(serialized);
    for (const [index, value] of chunks.entries()) {
      await SecureStore.setItemAsync(`${REGISTRY_KEY}.${String(index)}`, value);
    }
    await SecureStore.setItemAsync(REGISTRY_CHUNK_COUNT_KEY, String(chunks.length));
    for (let index = chunks.length; index < previous; index += 1) {
      await SecureStore.deleteItemAsync(`${REGISTRY_KEY}.${String(index)}`);
    }
    await SecureStore.deleteItemAsync(REGISTRY_KEY);
  }
}

function chunk(serialized: string): string[] {
  const chunks: string[] = [];
  let start = 0;
  while (start < serialized.length) {
    let end = Math.min(start + REGISTRY_CHUNK_LENGTH, serialized.length);
    if (end < serialized.length && isHighSurrogate(serialized.charCodeAt(end - 1))) end -= 1;
    chunks.push(serialized.slice(start, end));
    start = end;
  }
  return chunks.length === 0 ? [''] : chunks;
}

function isHighSurrogate(unit: number): boolean {
  return unit >= 0xd800 && unit <= 0xdbff;
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
  erasureMarker: profileErasureMarker,
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
  readonly blockIdentityMismatch: (
    subject: string,
    error: PersistenceIdentityMismatchError,
  ) => Promise<void>;
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
}: AuthenticatedLocalDataProviderProps): ReactElement {
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
  const lastProfileId = useRef<string | undefined>(undefined);
  if (profileId !== undefined) lastProfileId.current = profileId;

  useEffect(() => {
    let active = true;
    setApiSessionSubject(selectedProfileId);
    const transition = sessionTransition(
      sessionExpiredReason,
      selectedProfileId,
      lastProfileId.current,
    );
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
    async (subject: string, error: PersistenceIdentityMismatchError): Promise<void> => {
      await localDataLifecycle.blockIdentityMismatch(subject, error);
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

function sessionTransition(
  reason: SessionExpiredReason | undefined,
  profileId: string | undefined,
  expiredProfileId: string | undefined,
): Promise<unknown> {
  if (reason === undefined) {
    return profileId === undefined
      ? localDataLifecycle.clear()
      : localDataLifecycle.selectSubject(profileId);
  }
  if (expiredProfileId === undefined) return localDataLifecycle.clear();
  return reason === 'profile_erased'
    ? localDataLifecycle.handleProfileErased(expiredProfileId)
    : localDataLifecycle.handleSessionExpired(expiredProfileId);
}
