{% if context.auth_oidc %}import { OidcClient as ProviderClient } from '@baukit/auth-web';
{% elif context.auth_clerk %}import { ClerkWebClient as ProviderClient } from '@baukit/auth-web/clerk';
{% else %}import { WorkOsWebClient as ProviderClient } from '@baukit/auth-web/workos';
{% endif %}
import type { SessionExpiredEvent } from '@baukit/auth-web';

{% if context.auth_oidc %}import { PRODUCT_NAME } from './product';
{% endif %}

{% if context.auth_oidc %}const configuredIssuer: unknown = import.meta.env['VITE_OIDC_ISSUER'];
const configuredClientId: unknown = import.meta.env['VITE_OIDC_CLIENT_ID'];
const configuredAudience: unknown = import.meta.env['VITE_OIDC_AUDIENCE'];
const configuredResource: unknown = import.meta.env['VITE_OIDC_RESOURCE'];
const configuredScopes: unknown = import.meta.env['VITE_OIDC_SCOPES'];
const offlineAccess = import.meta.env['VITE_OIDC_OFFLINE_ACCESS'] !== 'false';

{% elif context.auth_clerk %}const configuredPublishableKey: unknown = import.meta.env['VITE_CLERK_PUBLISHABLE_KEY'];
{% else %}const configuredClientId: unknown = import.meta.env['VITE_WORKOS_CLIENT_ID'];
{% endif %}
let providerClient: ProviderClient | undefined;

function client(): ProviderClient {
  providerClient ??= new ProviderClient({
{% if context.auth_oidc %}    issuer:
      typeof configuredIssuer === 'string'
        ? configuredIssuer
        : `http://localhost:{{ context.keycloak_host_port }}/realms/${PRODUCT_NAME}`,
    clientId: typeof configuredClientId === 'string' ? configuredClientId : `${PRODUCT_NAME}-web`,
{% elif context.auth_clerk %}    publishableKey: typeof configuredPublishableKey === 'string' ? configuredPublishableKey : '',
{% else %}    clientId: typeof configuredClientId === 'string' ? configuredClientId : '',
{% endif %}    redirectUri: `${window.location.origin}/`,
{% if context.auth_oidc %}    scopes: typeof configuredScopes === 'string' && configuredScopes.trim()
      ? configuredScopes.trim().split(/\s+/u) : ['openid', 'profile', 'email'],
    ...(typeof configuredAudience === 'string' && configuredAudience.trim() ? { audience: configuredAudience } : {}),
    ...(typeof configuredResource === 'string' && configuredResource.trim() ? { resource: configuredResource } : {}),
    offlineAccess,
    storageKeyPrefix: `${PRODUCT_NAME}:oidc`,
{% endif %}  });
  return providerClient;
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
