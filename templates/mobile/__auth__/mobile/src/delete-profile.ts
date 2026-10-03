import Constants from 'expo-constants';
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import * as SQLite from 'expo-sqlite';
import { createApiRuntime } from '@baukit/api-runtime';
import {
  createProfileErasureClient,
  ProfileErasureKeyCleanupError,
  type ProfileErasureClient,
} from '@baukit/api-runtime/erasure';
import type { IdempotencyKeyStorage, StoredIdempotencyKey } from '@baukit/api-runtime/idempotency';
import { eraseProductProfile } from '@baukit/data-contracts';

import { authClient } from './auth';
import { createAppPreferenceRecordStore } from './record-store';

import { PRODUCT_NAME } from './product.js';

async function storageKey(slot: string): Promise<string> {
  const digest = await Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, slot);
  return `${PRODUCT_NAME}.profile-erasure.v1.` + digest;
}

function parseStoredKey(serialized: string | null): StoredIdempotencyKey | null {
  if (serialized === null) return null;
  const value: unknown = JSON.parse(serialized);
  if (typeof value !== 'object' || value === null)
    throw new TypeError('Invalid stored erasure key.');
  const key: unknown = Reflect.get(value, 'key');
  const createdAtMs: unknown = Reflect.get(value, 'createdAtMs');
  if (typeof key !== 'string' || typeof createdAtMs !== 'number' || !Number.isFinite(createdAtMs)) {
    throw new TypeError('Invalid stored erasure key.');
  }
  return { key, createdAtMs };
}

function durableKeys(): IdempotencyKeyStorage {
  return {
    get: async (slot) => parseStoredKey(await SecureStore.getItemAsync(await storageKey(slot))),
    set: async (slot, value) => {
      await SecureStore.setItemAsync(await storageKey(slot), JSON.stringify(value));
    },
    delete: async (slot) => {
      await SecureStore.deleteItemAsync(await storageKey(slot));
    },
  };
}

export async function createDeleteProfileClient(subject: string): Promise<ProfileErasureClient> {
  const token = await authClient.accessToken();
  const configuredBaseUrl: unknown = Constants.expoConfig?.extra?.['apiBaseUrl'];
  const runtime = createApiRuntime({
    baseUrl:
      typeof configuredBaseUrl === 'string'
        ? configuredBaseUrl
        : 'http://localhost:{{ context.api_host_port }}',
    environment: __DEV__ ? 'development' : 'production',
    tokenProvider: () => Promise.resolve(token ?? null),
    retry: false,
  });
  return createProfileErasureClient({
    fetch: runtime.fetch,
    account: subject,
    storage: durableKeys(),
    keyFactory: () => Crypto.randomUUID(),
  });
}

async function erasePreferences(subject: string): Promise<void> {
  const database = await SQLite.openDatabaseAsync(`${PRODUCT_NAME}-preferences.db`);
  try {
    const records = await createAppPreferenceRecordStore(database);
    await records.delete(subject);
  } finally {
    await database.closeAsync();
  }
}

export function deleteProfile(options: {
  readonly subject: string;
  readonly client: ProfileErasureClient;
  readonly eraseLocalPartition: () => Promise<void>;
  readonly resetPreferenceIdentity: () => Promise<void>;
}) {
  let keyCleanupError: ProfileErasureKeyCleanupError | undefined;
  return eraseProductProfile({
    eraseServerProfile: async () => {
      try {
        return await options.client.erase();
      } catch (cause) {
        if (!(cause instanceof ProfileErasureKeyCleanupError)) throw cause;
        keyCleanupError = cause;
        return cause.receipt;
      }
    },
    eraseLocalPartition: async () => {
      const results = await Promise.allSettled([
        options.eraseLocalPartition(),
        (async () => {
          try {
            await options.resetPreferenceIdentity();
          } finally {
            await erasePreferences(options.subject);
          }
        })(),
      ]);
      const failures = results.flatMap((result) =>
        result.status === 'rejected' ? [result.reason as unknown] : [],
      );
      if (keyCleanupError !== undefined) failures.push(keyCleanupError);
      if (failures.length > 0) throw new AggregateError(failures, 'Local profile erasure failed.');
    },
    signOut: () => authClient.clearSession(),
  });
}
