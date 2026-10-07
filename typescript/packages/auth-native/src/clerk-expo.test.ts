import { createElement, isValidElement, type PropsWithChildren, type ReactNode } from 'react';
import { expect, it, vi } from 'vitest';
import { createClerkExpoClient } from './clerk-expo.js';
import type { SecureStoragePort } from './index.js';

const sdk = vi.hoisted(() => ({
  loaded: true,
  user: { id: 'user_123' },
  getToken: vi.fn<() => Promise<string | null>>(),
  signOut: vi.fn<() => Promise<void>>(),
  hosted: vi.fn<() => Promise<{ createdSessionId: string | null }>>(),
}));
vi.mock('react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react')>();
  return {
    ...actual,
    useEffect: (effect: () => void) => {
      effect();
    },
  };
});
vi.mock('@clerk/expo', () => ({
  ClerkProvider: ({ children }: PropsWithChildren) => children,
  useAuth: () => ({ isLoaded: sdk.loaded }),
  useClerk: () => ({ user: sdk.user, session: { getToken: sdk.getToken }, signOut: sdk.signOut }),
}));
vi.mock('@clerk/expo/hosted-auth', () => ({
  useHostedAuth: () => ({ startHostedAuth: sdk.hosted }),
}));

interface ProviderProps {
  publishableKey: string;
  tokenCache: {
    getToken(key: string): Promise<string | null>;
    saveToken(key: string, value: string): Promise<void>;
    clearToken(key: string): Promise<void>;
  };
  children: ReactNode;
}
function functionComponent(value: unknown): value is (props: PropsWithChildren) => ReactNode {
  return typeof value === 'function';
}

it('binds hosted Clerk auth only after loading and delegates the SDK cache to secure storage', async () => {
  const values = new Map<string, string>();
  const storage: SecureStoragePort = {
    get: (key) => Promise.resolve(values.get(key) ?? null),
    set: (key, value) => {
      values.set(key, value);
      return Promise.resolve();
    },
    delete: (key) => {
      values.delete(key);
      return Promise.resolve();
    },
  };
  const { client, Provider } = createClerkExpoClient('pk_test', storage);
  const bind = vi.spyOn(client, 'bind');
  const tree = Provider({ children: createElement('div', {}, 'content') });
  if (!isValidElement<ProviderProps>(tree)) throw new Error('Expected Clerk provider');
  expect(tree.props.publishableKey).toBe('pk_test');
  const cache = tree.props.tokenCache;
  await cache.saveToken('session', 'secret');
  expect(await cache.getToken('session')).toBe('secret');
  await cache.clearToken('session');
  expect(await cache.getToken('session')).toBeNull();
  const bridge = tree.props.children;
  if (!isValidElement<PropsWithChildren>(bridge) || !functionComponent(bridge.type))
    throw new Error('Expected auth bridge');
  sdk.loaded = false;
  await bridge.type(bridge.props);
  expect(bind).not.toHaveBeenCalled();
  sdk.loaded = true;
  await bridge.type(bridge.props);
  const port = bind.mock.calls[0]?.[0];
  if (port === undefined) throw new Error('SDK port was not bound');
  expect(port.subject()).toBe('user_123');
  sdk.getToken.mockResolvedValue('access-token');
  expect(await port.getToken(true)).toBe('access-token');
  expect(sdk.getToken).toHaveBeenCalledWith({ skipCache: true });
  sdk.hosted.mockResolvedValue({ createdSessionId: 'session_123' });
  expect(await port.signIn()).toBe(true);
  sdk.hosted.mockResolvedValue({ createdSessionId: null });
  expect(await port.signIn()).toBe(false);
  sdk.signOut.mockResolvedValue();
  await port.signOut();
  expect(sdk.signOut).toHaveBeenCalledTimes(1);
});
