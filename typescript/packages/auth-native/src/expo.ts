import * as AuthSession from 'expo-auth-session';
import * as SecureStore from 'expo-secure-store';
import * as WebBrowser from 'expo-web-browser';

import {
  AUTHORIZATION_STATE_ENTROPY_BYTES,
  decoratedAuthorizationState,
} from './authorization-state.js';
import { NativeOidcClient } from './index.js';
import type {
  AuthorizationRequest,
  AuthorizationResult,
  BrowserFlowPort,
  FetchPort,
  NativeOidcConfig,
  NativeOidcEnvironment,
  SecureStoragePort,
} from './index.js';

export type RandomBytes = (size: number) => Uint8Array | Promise<Uint8Array>;

export interface ExpoBrowserFlowOptions {
  /**
   * Entropy for decorated authorization state, for example `getRandomBytesAsync`
   * from `expo-crypto`. Without it the flow ignores `stateDecoration`.
   */
  readonly randomBytes?: RandomBytes;
}

export interface ExpoOidcEnvironmentOptions extends ExpoBrowserFlowOptions {
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

/**
 * AuthSession browser flow with S256 PKCE. A request's `stateDecoration` goes in
 * front of a random nonce; when that fails, AuthSession's own state is used.
 */
export function createExpoBrowserFlow(options: ExpoBrowserFlowOptions = {}): BrowserFlowPort {
  return {
    async authorize(request) {
      const state = await decoratedState(request.stateDecoration, options.randomBytes);
      return authorizeWithAuthSession(request, state);
    },
    async endSession(request) {
      const result = await WebBrowser.openAuthSessionAsync(request.url, request.redirectUri);
      return result.type === 'success';
    },
  };
}

/** Creates thin Expo implementations of the native client's storage and browser ports. */
export function createExpoOidcEnvironment(
  options: ExpoOidcEnvironmentOptions = {},
): NativeOidcEnvironment {
  const storage = options.storage ?? createExpoSecureStorage(options.secureStoreOptions);
  const fetchImplementation = options.fetch ?? globalThis.fetch.bind(globalThis);
  return {
    fetch: fetchImplementation,
    storage,
    browser: createExpoBrowserFlow(options),
    now: options.now ?? (() => Date.now()),
  };
}

export function createExpoOidcClient(
  config: NativeOidcConfig,
  options: ExpoOidcEnvironmentOptions = {},
): NativeOidcClient {
  return new NativeOidcClient(config, createExpoOidcEnvironment(options));
}

async function decoratedState(
  decoration: readonly string[] | undefined,
  randomBytes: RandomBytes | undefined,
): Promise<string | undefined> {
  if (decoration === undefined || randomBytes === undefined) return undefined;
  try {
    const entropy = await randomBytes(AUTHORIZATION_STATE_ENTROPY_BYTES);
    return decoratedAuthorizationState(decoration, entropy);
  } catch {
    return undefined;
  }
}

async function authorizeWithAuthSession(
  request: AuthorizationRequest,
  state: string | undefined,
): Promise<AuthorizationResult> {
  const authRequest = new AuthSession.AuthRequest({
    clientId: request.clientId,
    redirectUri: request.redirectUri,
    responseType: AuthSession.ResponseType.Code,
    scopes: [...request.scopes],
    usePKCE: true,
    extraParams: {
      ...(request.audience === undefined ? {} : { audience: request.audience }),
      ...(request.resource === undefined ? {} : { resource: request.resource }),
    },
    ...(request.prompt === undefined ? {} : { prompt: AuthSession.Prompt.Login }),
    ...(state === undefined ? {} : { state }),
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
  };
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
