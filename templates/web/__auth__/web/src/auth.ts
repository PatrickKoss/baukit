import { OidcClient } from '@baukit/auth-web';
import type { SessionExpiredEvent } from '@baukit/auth-web';

import { PRODUCT_NAME } from './product';

const configuredIssuer: unknown = import.meta.env['VITE_OIDC_ISSUER'];
const configuredClientId: unknown = import.meta.env['VITE_OIDC_CLIENT_ID'];

let oidcClient: OidcClient | undefined;

function client(): OidcClient {
  oidcClient ??= new OidcClient({
    issuer:
      typeof configuredIssuer === 'string'
        ? configuredIssuer
        : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
    clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-web`,
    redirectUri: `${window.location.origin}/`,
    scopes: ['openid', 'profile', 'email'],
    offlineAccess: true,
    storageKeyPrefix: `${PRODUCT_NAME}:oidc`,
  });
  return oidcClient;
}

export const authClient = {
  hasSession: (): boolean => typeof window !== 'undefined' && client().hasSession(),
  login: (): Promise<void> => client().login(),
  handleCallback: (): Promise<boolean> =>
    typeof window === 'undefined' ? Promise.resolve(false) : client().handleCallback(),
  accessToken: (options: { readonly forceRefresh?: boolean } = {}): Promise<string | undefined> =>
    typeof window === 'undefined' ? Promise.resolve(undefined) : client().accessToken(options),
  subscribeSessionExpired: (listener: (event: SessionExpiredEvent) => void): (() => void) =>
    typeof window === 'undefined' ? () => undefined : client().subscribeSessionExpired(listener),
  logout: (): Promise<boolean> =>
    typeof window === 'undefined' ? Promise.resolve(false) : client().logout(),
  clearSession: (): void => {
    client().clearSession();
  },
};
