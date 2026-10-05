import * as SecureStore from 'expo-secure-store';

import { authStorage } from './auth-storage';
import { PRODUCT_NAME } from './product';

jest.mock('expo-secure-store', () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
  deleteItemAsync: jest.fn(),
}));

const storedValues = new Map<string, string>();

function validateKey(key: string): void {
  if (!/^[\w.-]+$/.test(key)) {
    throw new Error('Invalid SecureStore key');
  }
}

beforeEach(() => {
  storedValues.clear();
  jest.mocked(SecureStore.getItemAsync).mockImplementation((key) => {
    validateKey(key);
    return Promise.resolve(storedValues.get(key) ?? null);
  });
  jest.mocked(SecureStore.setItemAsync).mockImplementation((key, value) => {
    validateKey(key);
    storedValues.set(key, value);
    return Promise.resolve();
  });
  jest.mocked(SecureStore.deleteItemAsync).mockImplementation((key) => {
    validateKey(key);
    storedValues.delete(key);
    return Promise.resolve();
  });
});

it.each(['session', 'force-login'])('persists and removes the OIDC %s key', async (slot) => {
  const logicalKey = `${PRODUCT_NAME}.oidc.${slot}`;
  const physicalKey = `${PRODUCT_NAME}.oidc.${slot}`;

  await expect(authStorage.get(logicalKey)).resolves.toBeNull();
  await authStorage.set(logicalKey, 'stored-session');
  expect(storedValues.get(physicalKey)).toBe('stored-session');
  await expect(authStorage.get(logicalKey)).resolves.toBe('stored-session');
  await authStorage.delete(logicalKey);
  expect(storedValues.has(physicalKey)).toBe(false);
  await expect(authStorage.get(logicalKey)).resolves.toBeNull();
});
