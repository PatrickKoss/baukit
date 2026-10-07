import { createElement, useEffect, type ReactElement, type PropsWithChildren } from 'react';
import { ClerkProvider, useAuth, useClerk } from '@clerk/expo';
import { useHostedAuth } from '@clerk/expo/hosted-auth';
import type { SecureStoragePort } from './index.js';
import { ClerkNativeClient } from './clerk.js';

/** Creates a Clerk SDK provider and its product-facing client together. */
export function createClerkExpoClient(
  publishableKey: string,
  storage: SecureStoragePort,
): { client: ClerkNativeClient; Provider: (props: PropsWithChildren) => ReactElement } {
  const client = new ClerkNativeClient();
  function Bridge({ children }: PropsWithChildren) {
    const { isLoaded } = useAuth();
    const clerk = useClerk();
    const { startHostedAuth } = useHostedAuth();
    useEffect(() => {
      if (!isLoaded) return;
      client.bind({
        subject: () => clerk.user?.id,
        getToken: async (skipCache) => (await clerk.session?.getToken({ skipCache })) ?? null,
        signIn: async () => (await startHostedAuth()).createdSessionId !== null,
        signOut: () => clerk.signOut(),
      });
    }, [isLoaded, clerk, startHostedAuth]);
    return children;
  }
  function Provider({ children }: PropsWithChildren) {
    return createElement(ClerkProvider, {
      publishableKey,
      children: createElement(Bridge, {}, children),
      tokenCache: {
        getToken: (key: string) => storage.get(key),
        saveToken: (key: string, value: string) => storage.set(key, value),
        clearToken: (key: string) => storage.delete(key),
      },
    });
  }
  return { client, Provider };
}
