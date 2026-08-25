import { describe, expect, it, vi } from 'vitest';

vi.mock('expo-auth-session', () => ({
  AuthRequest: vi.fn(),
  Prompt: { Login: 'login' },
  ResponseType: { Code: 'code' },
}));

vi.mock('expo-secure-store', () => ({
  deleteItemAsync: vi.fn(),
  getItemAsync: vi.fn(),
  setItemAsync: vi.fn(),
}));

vi.mock('expo-web-browser', () => ({
  maybeCompleteAuthSession: vi.fn(),
  openAuthSessionAsync: vi.fn(),
}));

import type { SecureStoragePort } from './index.js';
import { createExpoOidcEnvironment } from './expo.js';

describe('createExpoOidcEnvironment', () => {
  it('preserves a product-owned storage port', () => {
    const storage: SecureStoragePort = {
      get: vi.fn(),
      set: vi.fn(),
      delete: vi.fn(),
    };

    const environment = createExpoOidcEnvironment({ storage });

    expect(environment.storage).toBe(storage);
  });
});
