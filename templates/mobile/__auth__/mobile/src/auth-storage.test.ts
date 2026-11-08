import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

import { authStorage, WebTokenStorageRefused } from "./auth-storage";
import { PRODUCT_NAME } from "./product";

jest.mock("expo-secure-store", () => ({
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
  deleteItemAsync: jest.fn(),
}));

const storedValues = new Map<string, string>();

function validateKey(key: string): void {
  if (!/^[\w.-]+$/.test(key)) {
    throw new Error("Invalid SecureStore key");
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

afterEach(() => {
  jest.restoreAllMocks();
});

it.each(["android", "ios"] as const)(
  "persists OIDC slots in SecureStore on %s",
  async (platform) => {
    jest.replaceProperty(Platform, "OS", platform);
    for (const slot of ["session", "force-login"]) {
      const key = `${PRODUCT_NAME}.oidc.${slot}`;

      await expect(authStorage.get(key)).resolves.toBeNull();
      await authStorage.set(key, "stored-session");
      expect(storedValues.get(key)).toBe("stored-session");
      await expect(authStorage.get(key)).resolves.toBe("stored-session");
      await authStorage.delete(key);
      expect(storedValues.has(key)).toBe(false);
    }
  },
);

it("refuses every call on web and touches neither SecureStore nor browser storage", async () => {
  jest.replaceProperty(Platform, "OS", "web");
  const setItem = jest.fn();
  const getItem = jest.fn();
  const removeItem = jest.fn();
  const browserStorage = { setItem, getItem, removeItem };
  const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: browserStorage });
  try {
    const key = `${PRODUCT_NAME}.oidc.session`;
    await expect(authStorage.set(key, "token")).rejects.toBeInstanceOf(WebTokenStorageRefused);
    await expect(authStorage.get(key)).rejects.toBeInstanceOf(WebTokenStorageRefused);
    await expect(authStorage.delete(key)).rejects.toBeInstanceOf(WebTokenStorageRefused);
  } finally {
    if (original === undefined) Reflect.deleteProperty(globalThis, "localStorage");
    else Object.defineProperty(globalThis, "localStorage", original);
  }
  expect(setItem).not.toHaveBeenCalled();
  expect(getItem).not.toHaveBeenCalled();
  expect(removeItem).not.toHaveBeenCalled();
  expect(SecureStore.setItemAsync).not.toHaveBeenCalled();
  expect(SecureStore.getItemAsync).not.toHaveBeenCalled();
  expect(SecureStore.deleteItemAsync).not.toHaveBeenCalled();
});
