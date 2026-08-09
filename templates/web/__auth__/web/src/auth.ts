import { OidcClient } from '@baukit/auth-web';

const configuredIssuer: unknown = import.meta.env['VITE_OIDC_ISSUER'];
const configuredClientId: unknown = import.meta.env['VITE_OIDC_CLIENT_ID'];

let oidcClient: OidcClient | undefined;

function client(): OidcClient {
  oidcClient ??= new OidcClient({
    issuer:
      typeof configuredIssuer === 'string'
        ? configuredIssuer
        : 'http://localhost:8081/realms/{{ context.app_name }}',
    clientId:
      typeof configuredClientId === 'string' ? configuredClientId : '{{ context.app_name }}-web',
    redirectUri: `${window.location.origin}/`,
    scopes: ['openid', 'profile', 'email'],
    offlineAccess: true,
    storageKeyPrefix: '{{ context.app_name }}:oidc',
  });
  return oidcClient;
}

export const authClient = {
  hasSession: (): boolean => typeof window !== 'undefined' && client().hasSession(),
  login: (): Promise<void> => client().login(),
  handleCallback: (): Promise<boolean> =>
    typeof window === 'undefined' ? Promise.resolve(false) : client().handleCallback(),
  accessToken: (): Promise<string | undefined> =>
    typeof window === 'undefined' ? Promise.resolve(undefined) : client().accessToken(),
  logout: (): Promise<boolean> =>
    typeof window === 'undefined' ? Promise.resolve(false) : client().logout(),
};
