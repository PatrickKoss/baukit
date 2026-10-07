import { afterEach, describe, expect, it, vi } from 'vitest';

interface AuthRequestConfig {
  readonly extraParams?: Readonly<Record<string, string>>;
  readonly state?: string;
  readonly prompt?: string;
}

const authRequestConfigs: AuthRequestConfig[] = [];
const DEFAULT_STATE = 'auth-session-state';

vi.mock('expo-auth-session', () => ({
  AuthRequest: class {
    public readonly state: string;
    public readonly codeVerifier = 'pkce-verifier';

    public constructor(config: AuthRequestConfig) {
      authRequestConfigs.push(config);
      this.state = config.state ?? DEFAULT_STATE;
    }

    public promptAsync() {
      return Promise.resolve({ type: 'success', params: { code: 'code', state: this.state } });
    }
  },
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

import type { AuthorizationRequest, SecureStoragePort } from './index.js';
import { createExpoBrowserFlow, createExpoOidcEnvironment } from './expo.js';

const request: AuthorizationRequest = {
  authorizationEndpoint: 'https://identity.example.test/authorize',
  clientId: 'product-mobile',
  redirectUri: 'product://oauth',
  scopes: ['openid'],
};

const zeroBytes = (size: number) => new Uint8Array(size);

afterEach(() => {
  authRequestConfigs.length = 0;
});

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

describe('createExpoBrowserFlow', () => {
  it('passes resource and audience to AuthSession while retaining PKCE and state', async () => {
    const result = await createExpoBrowserFlow().authorize({
      ...request,
      audience: 'https://api.example.test',
      resource: 'https://api.example.test',
    });
    expect(authRequestConfigs[0]?.extraParams).toEqual({
      audience: 'https://api.example.test',
      resource: 'https://api.example.test',
    });
    expect(result).toMatchObject({
      type: 'success',
      codeVerifier: 'pkce-verifier',
      expectedState: DEFAULT_STATE,
    });
  });
  it('prefixes the decoration to a random nonce', async () => {
    const browser = createExpoBrowserFlow({ randomBytes: zeroBytes });

    const result = await browser.authorize({ ...request, stateDecoration: ['ap1', 'd'] });

    const state = `ap1.d.${'00'.repeat(32)}`;
    expect(authRequestConfigs[0]?.state).toBe(state);
    expect(result).toMatchObject({ type: 'success', state, expectedState: state });
  });

  it('keeps the AuthSession state without a decoration or entropy source', async () => {
    await createExpoBrowserFlow({ randomBytes: zeroBytes }).authorize(request);
    await createExpoBrowserFlow().authorize({ ...request, stateDecoration: ['ap1', 'd'] });

    expect(authRequestConfigs.map((config) => config.state)).toEqual([undefined, undefined]);
  });

  it('falls back to the AuthSession state when decoration fails', async () => {
    const failing = createExpoBrowserFlow({
      randomBytes: () => Promise.reject(new Error('no entropy')),
    });
    const short = createExpoBrowserFlow({ randomBytes: () => new Uint8Array(4) });
    const malformed = createExpoBrowserFlow({ randomBytes: zeroBytes });

    const results = [
      await failing.authorize({ ...request, stateDecoration: ['ap1'] }),
      await short.authorize({ ...request, stateDecoration: ['ap1'] }),
      await malformed.authorize({ ...request, stateDecoration: ['a.b'] }),
    ];

    expect(authRequestConfigs.map((config) => config.state)).toEqual([
      undefined,
      undefined,
      undefined,
    ]);
    for (const result of results) {
      expect(result).toMatchObject({ type: 'success', expectedState: DEFAULT_STATE });
    }
  });
});
