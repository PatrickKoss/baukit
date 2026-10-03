import type { SecureStoragePort } from '@baukit/auth-native';
import * as SecureStore from 'expo-secure-store';

function secureStoreKey(key: string): string {
  return key.replaceAll(':', '.');
}

export const authStorage: SecureStoragePort = {
  get: (key) => SecureStore.getItemAsync(secureStoreKey(key)),
  set: (key, value) => SecureStore.setItemAsync(secureStoreKey(key), value),
  delete: (key) => SecureStore.deleteItemAsync(secureStoreKey(key)),
};
