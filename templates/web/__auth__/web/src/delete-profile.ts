import { createApiRuntime } from '@baukit/api-runtime';
import {
  createProfileErasureClient,
  ProfileErasureKeyCleanupError,
  type ProfileErasureClient,
} from '@baukit/api-runtime/erasure';
import type { IdempotencyKeyStorage, StoredIdempotencyKey } from '@baukit/api-runtime/idempotency';
import { eraseProductProfile } from '@baukit/data-contracts';

import { authClient } from './auth';

import { PRODUCT_NAME } from './product';

async function storageKey(slot: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(slot));
  const hex = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join(
    '',
  );
  return `${PRODUCT_NAME}:profile-erasure:v1:` + hex;
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
    get: async (slot) => parseStoredKey(localStorage.getItem(await storageKey(slot))),
    set: async (slot, value) => {
      localStorage.setItem(await storageKey(slot), JSON.stringify(value));
    },
    delete: async (slot) => {
      localStorage.removeItem(await storageKey(slot));
    },
  };
}

export async function createDeleteProfileClient(subject: string): Promise<ProfileErasureClient> {
  const token = await authClient.accessToken();
  const configuredBaseUrl: unknown = import.meta.env['VITE_API_URL'];
  const runtime = createApiRuntime({
    baseUrl:
      typeof configuredBaseUrl === 'string'
        ? configuredBaseUrl
        : 'http://localhost:{{ context.api_host_port }}',
    environment: import.meta.env.MODE,
    tokenProvider: () => Promise.resolve(token ?? null),
    retry: false,
  });
  return createProfileErasureClient({
    fetch: runtime.fetch,
    account: subject,
    storage: durableKeys(),
  });
}

export function deleteProfile(options: {
  readonly client: ProfileErasureClient;
  readonly eraseLocalPartition: () => Promise<void>;
  readonly onSignedOut: () => void;
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
      await options.eraseLocalPartition();
      if (keyCleanupError !== undefined) throw keyCleanupError;
    },
    signOut: () => {
      authClient.clearSession();
      options.onSignedOut();
      return Promise.resolve();
    },
  });
}
