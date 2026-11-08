import type { SecureStoragePort } from "@baukit/auth-native";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

/** The web build signs in through `@baukit/auth-web` and keeps no token in browser storage (ADR 0006). */
export class WebTokenStorageRefused extends Error {
  public constructor() {
    super("Token storage is not available in the web build.");
    this.name = "WebTokenStorageRefused";
  }
}

function nativeOnly(): void {
  if (Platform.OS === "web") throw new WebTokenStorageRefused();
}

export const authStorage: SecureStoragePort = {
  get: async (key) => {
    nativeOnly();
    return SecureStore.getItemAsync(key);
  },
  set: async (key, value) => {
    nativeOnly();
    await SecureStore.setItemAsync(key, value);
  },
  delete: async (key) => {
    nativeOnly();
    await SecureStore.deleteItemAsync(key);
  },
};
