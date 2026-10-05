import type { SecureStoragePort } from '@baukit/auth-native';
import * as SecureStore from 'expo-secure-store';

export const authStorage: SecureStoragePort = {
  get: (key) => SecureStore.getItemAsync(key),
  set: (key, value) => SecureStore.setItemAsync(key, value),
  delete: (key) => SecureStore.deleteItemAsync(key),
};
