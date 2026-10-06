import type { SecureStoragePort } from "@baukit/auth-native";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

export const authStorage: SecureStoragePort = {
  get: async (key) =>
    Platform.OS === "web"
      ? localStorage.getItem(key)
      : SecureStore.getItemAsync(key),
  set: async (key, value) => {
    if (Platform.OS === "web") {
      localStorage.setItem(key, value);
      return;
    }
    await SecureStore.setItemAsync(key, value);
  },
  delete: async (key) => {
    if (Platform.OS === "web") {
      localStorage.removeItem(key);
      return;
    }
    await SecureStore.deleteItemAsync(key);
  },
};
