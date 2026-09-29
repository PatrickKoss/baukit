import { randomUUID } from 'node:crypto';

/** Where a disposable development Keycloak runs and how to administer it. */
export interface KeycloakStack {
  readonly url: string;
  readonly realm: string;
  readonly adminUsername: string;
  readonly adminPassword: string;
  readonly webClientId: string;
}

/** The product's development defaults; `E2E_KEYCLOAK_*` variables override each one. */
export interface KeycloakStackDefaults {
  readonly url: string;
  readonly realm: string;
  readonly webClientId: string;
  /** Defaults to `admin`. */
  readonly adminUsername?: string;
  /** Defaults to `admin`. */
  readonly adminPassword?: string;
}

export interface KeycloakTestUser {
  readonly username: string;
  readonly password: string;
  readonly email: string;
  /** Keycloak user ID, which becomes the `sub` claim of the user's tokens. */
  readonly subject: string;
}

export interface KeycloakTestUserOptions {
  /** Defaults to `e2e-<uuid>`, so parallel tests never share an identity. */
  readonly username?: string;
  /** Defaults to a random password. */
  readonly password?: string;
  /** Defaults to `<username>@example.test`. */
  readonly email?: string;
}

export interface KeycloakRequestOptions {
  /** Defaults to the global `fetch`. */
  readonly fetch?: typeof fetch;
  /** Per-request timeout. Defaults to {@link DEFAULT_KEYCLOAK_REQUEST_TIMEOUT_MS}. */
  readonly timeoutMs?: number;
}

/** The part of a Playwright `Page` that {@link signInWithKeycloak} uses. */
export interface KeycloakLoginPage {
  waitForURL(url: (url: URL) => boolean, options?: { timeout?: number }): Promise<unknown>;
  locator(selector: string): {
    fill(value: string): Promise<unknown>;
    click(): Promise<unknown>;
  };
}

export interface KeycloakSignInOptions {
  /** How long to wait for the login page. Defaults to the page's own timeout. */
  readonly timeoutMs?: number;
}

export const DEFAULT_KEYCLOAK_REQUEST_TIMEOUT_MS = 30_000;

const HTTP_CREATED = 201;
const DEFAULT_ADMIN = 'admin';

/** Returns the stack from `E2E_KEYCLOAK_*` variables, falling back to the defaults. */
export function keycloakStack(
  defaults: KeycloakStackDefaults,
  environment: NodeJS.ProcessEnv = process.env,
): KeycloakStack {
  const url = environment['E2E_KEYCLOAK_URL'] ?? defaults.url;
  return {
    url: url.replace(/\/+$/u, ''),
    realm: environment['E2E_KEYCLOAK_REALM'] ?? defaults.realm,
    adminUsername:
      environment['E2E_KEYCLOAK_ADMIN_USERNAME'] ?? defaults.adminUsername ?? DEFAULT_ADMIN,
    adminPassword:
      environment['E2E_KEYCLOAK_ADMIN_PASSWORD'] ?? defaults.adminPassword ?? DEFAULT_ADMIN,
    webClientId: environment['E2E_KEYCLOAK_WEB_CLIENT_ID'] ?? defaults.webClientId,
  };
}

/** Creates a verified user with a password in the stack's realm. */
export async function createKeycloakTestUser(
  stack: KeycloakStack,
  user: KeycloakTestUserOptions = {},
  options: KeycloakRequestOptions = {},
): Promise<KeycloakTestUser> {
  const username = user.username ?? `e2e-${randomUUID()}`;
  const password = user.password ?? `E2e-${randomUUID()}`;
  const email = user.email ?? `${username}@example.test`;
  const admin = await adminRequester(stack, options);
  const response = await admin('users', {
    method: 'POST',
    body: {
      username,
      email,
      emailVerified: true,
      enabled: true,
      firstName: 'E2E',
      lastName: 'User',
      credentials: [{ type: 'password', value: password, temporary: false }],
    },
  });
  if (response.status !== HTTP_CREATED) {
    throw new Error(`Keycloak user creation failed with HTTP ${String(response.status)}.`);
  }
  const subject = response.headers.get('location')?.match(/\/([^/]+)$/u)?.[1];
  if (subject === undefined) {
    throw new Error('Keycloak user creation returned no user location.');
  }
  return { username, password, email, subject: decodeURIComponent(subject) };
}

/** Ends every Keycloak session of the user, so the next token refresh fails. */
export async function revokeKeycloakUserSessions(
  stack: KeycloakStack,
  subject: string,
  options: KeycloakRequestOptions = {},
): Promise<void> {
  const admin = await adminRequester(stack, options);
  const response = await admin(`users/${encodeURIComponent(subject)}/logout`, { method: 'POST' });
  if (!response.ok) {
    throw new Error(`Keycloak session revocation failed with HTTP ${String(response.status)}.`);
  }
}

interface KeycloakClient {
  readonly id: string;
  readonly redirectUris?: readonly string[];
  readonly webOrigins?: readonly string[];
}

/** Adds the origin to the web client's redirect URIs and web origins when the client lacks it. */
export async function allowKeycloakWebOrigin(
  stack: KeycloakStack,
  origin: string,
  options: KeycloakRequestOptions = {},
): Promise<void> {
  const admin = await adminRequester(stack, options);
  const lookup = await admin(`clients?clientId=${encodeURIComponent(stack.webClientId)}`);
  if (!lookup.ok) {
    throw new Error(`Keycloak client lookup failed with HTTP ${String(lookup.status)}.`);
  }
  const [client] = (await lookup.json()) as readonly (KeycloakClient & Record<string, unknown>)[];
  if (client === undefined) {
    throw new Error(`Keycloak realm ${stack.realm} has no ${stack.webClientId} client.`);
  }
  const redirectUri = `${origin}/*`;
  if (client.redirectUris?.includes(redirectUri) && client.webOrigins?.includes(origin)) {
    return;
  }
  const update = await admin(`clients/${encodeURIComponent(client.id)}`, {
    method: 'PUT',
    body: {
      ...client,
      redirectUris: withEntry(client.redirectUris, redirectUri),
      webOrigins: withEntry(client.webOrigins, origin),
    },
  });
  if (!update.ok) {
    throw new Error(`Keycloak client update failed with HTTP ${String(update.status)}.`);
  }
}

/** Waits for the Keycloak login page, fills it by its stable `keycloak.v2` IDs, and submits. */
export async function signInWithKeycloak(
  loginPage: KeycloakLoginPage,
  user: Pick<KeycloakTestUser, 'username' | 'password'>,
  stack: KeycloakStack,
  options: KeycloakSignInOptions = {},
): Promise<void> {
  const origin = new URL(stack.url).origin;
  const isKeycloak = (url: URL): boolean => url.origin === origin;
  await (options.timeoutMs === undefined
    ? loginPage.waitForURL(isKeycloak)
    : loginPage.waitForURL(isKeycloak, { timeout: options.timeoutMs }));
  await loginPage.locator('#username').fill(user.username);
  await loginPage.locator('#password').fill(user.password);
  await loginPage.locator('#kc-login').click();
}

interface AdminRequest {
  readonly method?: 'GET' | 'POST' | 'PUT';
  readonly body?: unknown;
}

type AdminRequester = (path: string, request?: AdminRequest) => Promise<Response>;

async function adminRequester(
  stack: KeycloakStack,
  options: KeycloakRequestOptions,
): Promise<AdminRequester> {
  const fetcher = options.fetch ?? fetch;
  const timeoutMs = options.timeoutMs ?? DEFAULT_KEYCLOAK_REQUEST_TIMEOUT_MS;
  const token = await adminAccessToken(stack, fetcher, timeoutMs);
  const realmUrl = `${stack.url}/admin/realms/${encodeURIComponent(stack.realm)}`;
  return (path, request = {}) =>
    fetcher(`${realmUrl}/${path}`, {
      method: request.method ?? 'GET',
      headers: {
        authorization: `Bearer ${token}`,
        ...(request.body === undefined ? {} : { 'content-type': 'application/json' }),
      },
      ...(request.body === undefined ? {} : { body: JSON.stringify(request.body) }),
      signal: AbortSignal.timeout(timeoutMs),
    });
}

async function adminAccessToken(
  stack: KeycloakStack,
  fetcher: typeof fetch,
  timeoutMs: number,
): Promise<string> {
  const response = await fetcher(`${stack.url}/realms/master/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: 'admin-cli',
      grant_type: 'password',
      username: stack.adminUsername,
      password: stack.adminPassword,
    }),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new Error(`Keycloak admin sign-in failed with HTTP ${String(response.status)}.`);
  }
  const body = (await response.json()) as { readonly access_token?: unknown };
  if (typeof body.access_token !== 'string') {
    throw new Error('Keycloak admin sign-in returned no access token.');
  }
  return body.access_token;
}

function withEntry(entries: readonly string[] | undefined, entry: string): string[] {
  const current = entries ?? [];
  return current.includes(entry) ? [...current] : [...current, entry];
}
