import {
  NativeOidcClient,
  OidcError,
  type NativeOidcConfig,
  type NativeOidcEnvironment,
  type NativeTokenProtocol,
} from './index.js';

/** Reads scheduling claims only. The backend verifies the token before accepting it. */
export function tokenTiming(token: string): { subject: string; expiresAt: number } {
  try {
    const encoded = token.split('.')[1];
    if (encoded === undefined) throw new OidcError('invalid_token_response');
    const value: unknown = JSON.parse(
      globalThis.atob(encoded.replaceAll('-', '+').replaceAll('_', '/')),
    );
    if (
      typeof value !== 'object' ||
      value === null ||
      !('sub' in value) ||
      !('exp' in value) ||
      typeof value.sub !== 'string' ||
      value.sub.length === 0 ||
      typeof value.exp !== 'number' ||
      !Number.isFinite(value.exp)
    ) {
      throw new OidcError('invalid_token_response');
    }
    return { subject: value.sub, expiresAt: value.exp * 1000 };
  } catch {
    throw new OidcError('invalid_token_response');
  }
}

function workosProtocol(
  config: NativeOidcConfig,
  environment: NativeOidcEnvironment,
): NativeTokenProtocol {
  const base = config.issuer.replace(/\/+$/u, '');
  return {
    async logout(session, redirectUri) {
      const encoded = session.accessToken.split('.')[1];
      if (encoded === undefined) throw new OidcError('invalid_token_response');
      const claims: unknown = JSON.parse(
        globalThis.atob(encoded.replaceAll('-', '+').replaceAll('_', '/')),
      );
      if (
        typeof claims !== 'object' ||
        claims === null ||
        !('sid' in claims) ||
        typeof claims.sid !== 'string' ||
        claims.sid.length === 0
      )
        throw new OidcError('invalid_token_response');
      const url = new URL(`${base}/user_management/sessions/logout`);
      url.searchParams.set('session_id', claims.sid);
      url.searchParams.set('return_to', redirectUri);
      return environment.browser.endSession({ url: url.toString(), redirectUri });
    },
    discover: () =>
      Promise.resolve({
        issuer: base,
        authorizationEndpoint: `${base}/user_management/authorize?provider=authkit`,
        tokenEndpoint: `${base}/user_management/authenticate`,
        userInfoEndpoint: `${base}/user_management/users`,
      }),
    async exchange(endpoint, body) {
      const payload = Object.fromEntries(body);
      delete payload['scope'];
      const response = await environment.fetch(endpoint, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!response.ok) return response;
      const value: unknown = await response.json();
      if (
        typeof value !== 'object' ||
        value === null ||
        !('access_token' in value) ||
        typeof value.access_token !== 'string'
      )
        throw new OidcError('invalid_token_response');
      const timing = tokenTiming(value.access_token);
      return new Response(
        JSON.stringify({
          ...value,
          expires_in: Math.max(0, (timing.expiresAt - environment.now()) / 1000),
        }),
        {
          status: response.status,
          headers: { 'content-type': 'application/json' },
        },
      );
    },
    subject: (token) => Promise.resolve(tokenTiming(token).subject),
  };
}

/** AuthKit public-client PKCE, without an API key in the application bundle. */
export function createWorkOsNativeClient(
  config: NativeOidcConfig,
  environment: NativeOidcEnvironment,
): NativeOidcClient {
  return new NativeOidcClient(config, {
    ...environment,
    protocol: workosProtocol(config, environment),
  });
}
