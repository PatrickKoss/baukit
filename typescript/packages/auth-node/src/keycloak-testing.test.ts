import type { Page } from '@playwright/test';
import { describe, expect, it } from 'vitest';

import {
  allowKeycloakWebOrigin,
  createKeycloakTestUser,
  keycloakStack,
  revokeKeycloakUserSessions,
  signInWithKeycloak,
  type KeycloakLoginPage,
} from './keycloak-testing.js';

const STACK = keycloakStack(
  { url: 'http://localhost:8081/', realm: 'notes', webClientId: 'notes-web' },
  {},
);
const TOKEN_URL = 'http://localhost:8081/realms/master/protocol/openid-connect/token';
const ADMIN_URL = 'http://localhost:8081/admin/realms/notes';

interface RecordedRequest {
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  readonly body: string;
}

function bodyText(body: BodyInit | null | undefined): string {
  if (typeof body === 'string') return body;
  return body instanceof URLSearchParams ? body.toString() : '';
}

function fakeKeycloak(responses: Record<string, () => Response>): {
  readonly fetch: typeof fetch;
  readonly requests: RecordedRequest[];
} {
  const requests: RecordedRequest[] = [];
  const fetcher: typeof fetch = (input, init) => {
    const method = init?.method ?? 'GET';
    const url = input instanceof Request ? input.url : input.toString();
    requests.push({ url, method, headers: new Headers(init?.headers), body: bodyText(init?.body) });
    const respond = responses[`${method} ${url}`];
    if (respond === undefined) throw new Error(`unexpected ${method} ${url}`);
    return Promise.resolve(respond());
  };
  return { fetch: fetcher, requests };
}

const adminToken = (): Response => Response.json({ access_token: 'admin-token' });

describe('keycloakStack', () => {
  it('uses the defaults and admin/admin without variables', () => {
    expect(STACK).toEqual({
      url: 'http://localhost:8081',
      realm: 'notes',
      adminUsername: 'admin',
      adminPassword: 'admin',
      webClientId: 'notes-web',
    });
  });

  it('lets E2E_KEYCLOAK_* variables override every field', () => {
    const stack = keycloakStack(
      { url: 'http://localhost:8081', realm: 'notes', webClientId: 'notes-web' },
      {
        E2E_KEYCLOAK_URL: 'https://id.example.test//',
        E2E_KEYCLOAK_REALM: 'staging',
        E2E_KEYCLOAK_ADMIN_USERNAME: 'root',
        E2E_KEYCLOAK_ADMIN_PASSWORD: 'secret',
        E2E_KEYCLOAK_WEB_CLIENT_ID: 'staging-web',
      },
    );
    expect(stack).toEqual({
      url: 'https://id.example.test',
      realm: 'staging',
      adminUsername: 'root',
      adminPassword: 'secret',
      webClientId: 'staging-web',
    });
  });
});

describe('createKeycloakTestUser', () => {
  it('creates a verified random user and returns the subject from the location', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users`]: () =>
        new Response(null, {
          status: 201,
          headers: { location: `${ADMIN_URL}/users/0f4c-subject` },
        }),
    });

    const user = await createKeycloakTestUser(STACK, {}, { fetch: keycloak.fetch });

    expect(user.subject).toBe('0f4c-subject');
    expect(user.username).toMatch(/^e2e-[0-9a-f-]{36}$/u);
    expect(user.email).toBe(`${user.username}@example.test`);
    const [login, create] = keycloak.requests;
    expect(Object.fromEntries(new URLSearchParams(login?.body))).toEqual({
      client_id: 'admin-cli',
      grant_type: 'password',
      username: 'admin',
      password: 'admin',
    });
    expect(create?.headers.get('authorization')).toBe('Bearer admin-token');
    expect(JSON.parse(create?.body ?? '')).toMatchObject({
      username: user.username,
      email: user.email,
      emailVerified: true,
      enabled: true,
      credentials: [{ type: 'password', value: user.password, temporary: false }],
    });
  });

  it('keeps a caller-chosen identity', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users`]: () =>
        new Response(null, { status: 201, headers: { location: `${ADMIN_URL}/users/abc` } }),
    });

    const user = await createKeycloakTestUser(
      STACK,
      { username: 'ada@example.com', email: 'ada@example.com', password: 'pw' },
      { fetch: keycloak.fetch },
    );

    expect(user).toEqual({
      username: 'ada@example.com',
      email: 'ada@example.com',
      password: 'pw',
      subject: 'abc',
    });
  });

  it('names the failing step and status without a response body', async () => {
    const refused = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: () => new Response('bad credentials', { status: 401 }),
    });
    await expect(createKeycloakTestUser(STACK, {}, { fetch: refused.fetch })).rejects.toThrow(
      'Keycloak admin sign-in failed with HTTP 401.',
    );

    const conflict = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users`]: () => new Response(null, { status: 409 }),
    });
    await expect(createKeycloakTestUser(STACK, {}, { fetch: conflict.fetch })).rejects.toThrow(
      'Keycloak user creation failed with HTTP 409.',
    );

    const noLocation = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users`]: () => new Response(null, { status: 201 }),
    });
    await expect(createKeycloakTestUser(STACK, {}, { fetch: noLocation.fetch })).rejects.toThrow(
      'Keycloak user creation returned no user location.',
    );
  });
});

describe('revokeKeycloakUserSessions', () => {
  it('logs the user out through the admin API', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users/abc/logout`]: () => new Response(null, { status: 204 }),
    });

    await revokeKeycloakUserSessions(STACK, 'abc', { fetch: keycloak.fetch });

    expect(keycloak.requests.map((request) => request.method)).toEqual(['POST', 'POST']);
  });

  it('rejects a failed revocation', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [`POST ${ADMIN_URL}/users/abc/logout`]: () => new Response(null, { status: 404 }),
    });

    await expect(
      revokeKeycloakUserSessions(STACK, 'abc', { fetch: keycloak.fetch }),
    ).rejects.toThrow('Keycloak session revocation failed with HTTP 404.');
  });
});

describe('allowKeycloakWebOrigin', () => {
  const lookup = `GET ${ADMIN_URL}/clients?clientId=notes-web`;

  it('adds a missing origin and keeps the rest of the client', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [lookup]: () =>
        Response.json([
          {
            id: 'client-1',
            clientId: 'notes-web',
            redirectUris: ['http://localhost:5173/*'],
            webOrigins: ['http://localhost:5173'],
          },
        ]),
      [`PUT ${ADMIN_URL}/clients/client-1`]: () => new Response(null, { status: 204 }),
    });

    await allowKeycloakWebOrigin(STACK, 'http://localhost:5183', { fetch: keycloak.fetch });

    const update = keycloak.requests.at(-1);
    expect(update?.headers.get('content-type')).toBe('application/json');
    expect(JSON.parse(update?.body ?? '')).toEqual({
      id: 'client-1',
      clientId: 'notes-web',
      redirectUris: ['http://localhost:5173/*', 'http://localhost:5183/*'],
      webOrigins: ['http://localhost:5173', 'http://localhost:5183'],
    });
  });

  it('skips the update when the client already allows the origin', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [lookup]: () =>
        Response.json([
          {
            id: 'client-1',
            redirectUris: ['http://localhost:5173/*'],
            webOrigins: ['http://localhost:5173'],
          },
        ]),
    });

    await allowKeycloakWebOrigin(STACK, 'http://localhost:5173', { fetch: keycloak.fetch });

    expect(keycloak.requests).toHaveLength(2);
  });

  it('rejects a realm without the web client', async () => {
    const keycloak = fakeKeycloak({
      [`POST ${TOKEN_URL}`]: adminToken,
      [lookup]: () => Response.json([]),
    });

    await expect(
      allowKeycloakWebOrigin(STACK, 'http://localhost:5173', { fetch: keycloak.fetch }),
    ).rejects.toThrow('Keycloak realm notes has no notes-web client.');
  });
});

describe('signInWithKeycloak', () => {
  function fakePage(): { readonly page: KeycloakLoginPage; readonly steps: string[] } {
    const steps: string[] = [];
    const onKeycloak = new URL('http://localhost:8081/realms/notes/login');
    const onApp = new URL('http://localhost:5173/');
    const page: KeycloakLoginPage = {
      waitForURL: (matches, options) => {
        const timeout = options?.timeout === undefined ? 'default' : String(options.timeout);
        steps.push(`wait ${String(matches(onKeycloak))} ${String(matches(onApp))} ${timeout}`);
        return Promise.resolve();
      },
      locator: (selector) => ({
        fill: (value) => {
          steps.push(`fill ${selector} ${value}`);
          return Promise.resolve();
        },
        click: () => {
          steps.push(`click ${selector}`);
          return Promise.resolve();
        },
      }),
    };
    return { page, steps };
  }

  it('waits for the Keycloak origin and submits the form by its IDs', async () => {
    const { page, steps } = fakePage();

    await signInWithKeycloak(page, { username: 'ada', password: 'pw' }, STACK, {
      timeoutMs: 60_000,
    });
    await signInWithKeycloak(page, { username: 'ada', password: 'pw' }, STACK);

    expect(steps).toEqual([
      'wait true false 60000',
      'fill #username ada',
      'fill #password pw',
      'click #kc-login',
      'wait true false default',
      'fill #username ada',
      'fill #password pw',
      'click #kc-login',
    ]);
  });

  it('accepts a Playwright page', () => {
    const asLoginPage = (page: Page): KeycloakLoginPage => page;
    expect(asLoginPage).toBeTypeOf('function');
  });
});
