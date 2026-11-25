import { beforeEach, describe, expect, it, jest } from '@jest/globals';
import { InMemoryScopedPersistenceRegistryStore } from '@baukit/data-contracts';

const mockKeys = new Map<string, string>();

jest.mock('expo-crypto', () => {
  const { createHash } = jest.requireActual<typeof import('node:crypto')>('node:crypto');
  return {
    CryptoDigestAlgorithm: { SHA256: 'SHA-256' },
    digestStringAsync: (_algorithm: string, text: string) =>
      Promise.resolve(createHash('sha256').update(text).digest('hex')),
  };
});
jest.mock('expo-secure-store', () => ({
  getItemAsync: (key: string) => Promise.resolve(mockKeys.get(key) ?? null),
  setItemAsync: (key: string, value: string) => {
    mockKeys.set(key, value);
    return Promise.resolve();
  },
  deleteItemAsync: (key: string) => {
    mockKeys.delete(key);
    return Promise.resolve();
  },
}));

import { profileErasureMarker } from './erasure-marker';
import { createProductPersistenceLifecycle } from './persistence-lifecycle';

beforeEach(() => {
  mockKeys.clear();
});

describe('profile erasure marker', () => {
  it('finishes an interrupted local wipe on the next selection instead of opening', async () => {
    await profileErasureMarker.mark('account-e');
    const open = jest.fn(() => Promise.resolve({ close: () => Promise.resolve() }));
    const erase = jest.fn(() => Promise.resolve());
    const lifecycle = createProductPersistenceLifecycle({
      registry: new InMemoryScopedPersistenceRegistryStore(),
      digest: () => Promise.resolve('e'.repeat(64)),
      erasureMarker: profileErasureMarker,
      open,
      erase,
    });

    await lifecycle.selectSubject('account-e');

    expect(open).not.toHaveBeenCalled();
    expect(erase).toHaveBeenCalledTimes(1);
    expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'profile-erased' });
    expect(mockKeys.size).toBe(0);
  });

  it('uses only SecureStore-safe key characters', async () => {
    await profileErasureMarker.mark('user@example.com/äö');
    for (const key of mockKeys.keys()) expect(key).toMatch(/^[\w.-]+$/u);
  });
});
