import { randomUUID } from 'node:crypto';
import type { APIRequestContext, Page } from '@playwright/test';

export interface KeycloakStack {
  readonly url: string;
  readonly realm: string;
  readonly adminUsername: string;
  readonly adminPassword: string;
}

export interface KeycloakTestUser {
  readonly username: string;
  readonly password: string;
  /** Keycloak user ID, which becomes the `sub` claim of the user's tokens. */
  readonly subject: string;
}

const HTTP_CREATED = 201;

/** The composed development Keycloak unless `E2E_KEYCLOAK_*` variables point elsewhere. */
export function keycloakStack(environment: NodeJS.ProcessEnv = process.env): KeycloakStack {
  const url = environment['E2E_KEYCLOAK_URL'] ?? 'http://localhost:{{ context.keycloak_host_port }}';
  return {
    url: url.replace(/\/+$/u, ''),
    realm: environment['E2E_KEYCLOAK_REALM'] ?? '{{ context.app_name }}',
    adminUsername: environment['E2E_KEYCLOAK_ADMIN_USERNAME'] ?? 'admin',
    adminPassword: environment['E2E_KEYCLOAK_ADMIN_PASSWORD'] ?? 'admin',
  };
}

async function adminAccessToken(request: APIRequestContext, stack: KeycloakStack): Promise<string> {
  const response = await request.post(`${stack.url}/realms/master/protocol/openid-connect/token`, {
    form: {
      client_id: 'admin-cli',
      grant_type: 'password',
      username: stack.adminUsername,
      password: stack.adminPassword,
    },
  });
  if (!response.ok()) {
    throw new Error(`Keycloak admin sign-in failed with HTTP ${String(response.status())}.`);
  }
  const body = (await response.json()) as { readonly access_token?: unknown };
  if (typeof body.access_token !== 'string') {
    throw new Error('Keycloak admin sign-in returned no access token.');
  }
  return body.access_token;
}

/** Creates a verified user with a random password, so parallel tests never share an identity. */
export async function createKeycloakTestUser(
  request: APIRequestContext,
  stack: KeycloakStack = keycloakStack(),
): Promise<KeycloakTestUser> {
  const username = `e2e-${randomUUID()}`;
  const password = `E2e-${randomUUID()}`;
  const token = await adminAccessToken(request, stack);
  const response = await request.post(
    `${stack.url}/admin/realms/${encodeURIComponent(stack.realm)}/users`,
    {
      headers: { authorization: `Bearer ${token}` },
      data: {
        username,
        email: `${username}@example.test`,
        emailVerified: true,
        enabled: true,
        firstName: 'E2E',
        lastName: 'User',
        credentials: [{ type: 'password', value: password, temporary: false }],
      },
    },
  );
  if (response.status() !== HTTP_CREATED) {
    throw new Error(`Keycloak user creation failed with HTTP ${String(response.status())}.`);
  }
  const subject = response.headers()['location']?.match(/\/([^/]+)$/u)?.[1];
  if (subject === undefined) {
    throw new Error('Keycloak user creation returned no user location.');
  }
  return { username, password, subject };
}

/** Fills the Keycloak login form by its stable `keycloak.v2` IDs and submits it. */
export async function signInWithKeycloak(
  loginPage: Page,
  user: KeycloakTestUser,
  stack: KeycloakStack = keycloakStack(),
): Promise<void> {
  const origin = new URL(stack.url).origin;
  await loginPage.waitForURL((url) => url.origin === origin);
  await loginPage.locator('#username').fill(user.username);
  await loginPage.locator('#password').fill(user.password);
  await loginPage.locator('#kc-login').click();
}
