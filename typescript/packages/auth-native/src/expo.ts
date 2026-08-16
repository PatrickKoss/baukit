import * as AuthSession from 'expo-auth-session';
import * as SecureStore from 'expo-secure-store';
import * as WebBrowser from 'expo-web-browser';

import { NativeOidcClient } from './index.js';
import type {
  AuthorizationResult,
  BrowserFlowPort,
  FetchPort,
  NativeOidcConfig,
  NativeOidcEnvironment,
  SecureStoragePort,
} from './index.js';

export interface ExpoOidcEnvironmentOptions {
  readonly fetch?: FetchPort;
  readonly now?: () => number;
  /** Overrides SecureStore for universal Expo apps or product-owned migration adapters. */
  readonly storage?: SecureStoragePort;
  readonly secureStoreOptions?: SecureStore.SecureStoreOptions;
}

/** Completes an Expo web auth redirect when the module is used on web. */
export function completeExpoAuthSession(): void {
  WebBrowser.maybeCompleteAuthSession();
}

/** Creates thin Expo implementations of the native client's storage and browser ports. */
export function createExpoOidcEnvironment(
  options: ExpoOidcEnvironmentOptions = {},
): NativeOidcEnvironment {
  const storage = options.storage ?? createExpoSecureStorage(options.secureStoreOptions);
  const browser: BrowserFlowPort = {
    async authorize(request) {
      const authRequest = new AuthSession.AuthRequest({
        clientId: request.clientId,
        redirectUri: request.redirectUri,
        responseType: AuthSession.ResponseType.Code,
        scopes: [...request.scopes],
        usePKCE: true,
        ...(request.prompt === undefined ? {} : { prompt: AuthSession.Prompt.Login }),
      });
      const result = await authRequest.promptAsync({
        authorizationEndpoint: request.authorizationEndpoint,
      });
      if (result.type === 'cancel' || result.type === 'dismiss') {
        return { type: result.type };
      }
      if (result.type !== 'success') {
        return { type: 'error' };
      }
      return {
        type: 'success',
        code: result.params['code'] ?? '',
        state: result.params['state'] ?? '',
        expectedState: authRequest.state,
        codeVerifier: authRequest.codeVerifier ?? '',
      } satisfies AuthorizationResult;
    },
    async endSession(request) {
      const result = await WebBrowser.openAuthSessionAsync(request.url, request.redirectUri);
      return result.type === 'success';
    },
  };
  const fetchImplementation = options.fetch ?? globalThis.fetch.bind(globalThis);
  return {
    fetch: fetchImplementation,
    storage,
    browser,
    now: options.now ?? (() => Date.now()),
  };
}

export function createExpoOidcClient(
  config: NativeOidcConfig,
  options: ExpoOidcEnvironmentOptions = {},
): NativeOidcClient {
  return new NativeOidcClient(config, createExpoOidcEnvironment(options));
}

function createExpoSecureStorage(
  options: SecureStore.SecureStoreOptions | undefined,
): SecureStoragePort {
  return {
    get: (key) => SecureStore.getItemAsync(key, options),
    set: (key, value) => SecureStore.setItemAsync(key, value, options),
    delete: (key) => SecureStore.deleteItemAsync(key, options),
  };
}
