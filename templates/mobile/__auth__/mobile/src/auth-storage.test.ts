import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

import { authStorage } from "./auth-storage";
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

const browserValues = new Map<string, string>();
const browserStorage = {
  getItem: jest.fn((key: string) => browserValues.get(key) ?? null),
  setItem: jest.fn((key: string, value: string) => {
    browserValues.set(key, value);
  }),
  removeItem: jest.fn((key: string) => {
    browserValues.delete(key);
  }),
};
const originalStorage = Object.getOwnPropertyDescriptor(
  globalThis,
  "localStorage",
);
beforeEach(() => {
  browserValues.clear();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: browserStorage,
  });
});
afterEach(() => {
  jest.restoreAllMocks();
  if (originalStorage === undefined)
    Reflect.deleteProperty(globalThis, "localStorage");
  else Object.defineProperty(globalThis, "localStorage", originalStorage);
});

it.each(["android", "ios", "web"] as const)(
  "persists OIDC slots on %s",
  async (platform) => {
    jest.replaceProperty(Platform, "OS", platform);
    for (const slot of ["session", "force-login"]) {
      const logicalKey = `${PRODUCT_NAME}.oidc.${slot}`;
      const physicalKey = `${PRODUCT_NAME}.oidc.${slot}`;

      await expect(authStorage.get(logicalKey)).resolves.toBeNull();
      await authStorage.set(logicalKey, "stored-session");
      expect(
        (platform === "web" ? browserValues : storedValues).get(physicalKey),
      ).toBe("stored-session");
      await expect(authStorage.get(logicalKey)).resolves.toBe("stored-session");
      await authStorage.delete(logicalKey);
      expect(
        (platform === "web" ? browserValues : storedValues).has(physicalKey),
      ).toBe(false);
      await expect(authStorage.get(logicalKey)).resolves.toBeNull();
    }
    if (platform === "web") {
      expect(SecureStore.getItemAsync).not.toHaveBeenCalled();
      expect(SecureStore.setItemAsync).not.toHaveBeenCalled();
      expect(SecureStore.deleteItemAsync).not.toHaveBeenCalled();
    } else {
      expect(browserStorage.getItem).not.toHaveBeenCalled();
      expect(browserStorage.setItem).not.toHaveBeenCalled();
      expect(browserStorage.removeItem).not.toHaveBeenCalled();
    }
  },
);

it("reports browser storage failures", async () => {
  jest.replaceProperty(Platform, "OS", "web");
  browserStorage.setItem.mockImplementationOnce(() => {
    throw new Error("Storage unavailable");
  });
  await expect(authStorage.set("session", "value")).rejects.toThrow(
    "Storage unavailable",
  );
});
