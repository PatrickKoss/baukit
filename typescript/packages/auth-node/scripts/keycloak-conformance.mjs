import { execFile } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';

import { chromium } from '@playwright/test';

import { DeviceFlowClient } from '../dist/index.js';
import { keycloakStack, signInWithKeycloak } from '../dist/keycloak-testing.js';

const KEYCLOAK_IMAGE = 'quay.io/keycloak/keycloak:26.7.0';
const REALM = 'baukit-auth-node-conformance';
const CLIENT_ID = 'baukit-auth-node-conformance';
const USERNAME = 'conformance-user';
const PASSWORD = 'conformance-password';
const run = promisify(execFile);
const containerName = `baukit-auth-node-keycloak-${String(process.pid)}`;
const temporaryDirectory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-keycloak-'));
let browser;

try {
  await run('docker', [
    'run',
    '--detach',
    '--rm',
    '--name',
    containerName,
    '--publish',
    '127.0.0.1::8080',
    '--env',
    'KC_BOOTSTRAP_ADMIN_USERNAME=admin',
    '--env',
    'KC_BOOTSTRAP_ADMIN_PASSWORD=admin',
    KEYCLOAK_IMAGE,
    'start-dev',
  ]);
  const { stdout } = await run('docker', ['port', containerName, '8080/tcp']);
  const baseUrl = `http://${stdout.trim()}`;
  await waitUntilReady(baseUrl);
  await configureRealm(baseUrl);

  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const statuses = [];
  const auth = new DeviceFlowClient({
    issuer: `${baseUrl}/realms/${REALM}`,
    clientId: CLIENT_ID,
    scopes: ['openid', 'profile', 'offline_access'],
    cache: {
      namespace: 'baukit-auth-node-keycloak',
      path: join(temporaryDirectory, 'tokens.json'),
    },
    endpointPolicy: { allowLoopbackHttp: true },
    requestTimeoutMs: 10_000,
    loginTimeoutMs: 120_000,
    refreshLeewaySeconds: 0,
  });

  const loggedIn = await auth.login({
    presentation: {
      showVerification: async ({ verificationUriComplete }) => {
        if (verificationUriComplete === undefined) {
          throw new Error('Keycloak did not return verification_uri_complete.');
        }
        await approveInBrowser(page, verificationUriComplete);
      },
      showStatus: (status) => {
        statuses.push(status);
      },
    },
  });
  assert(loggedIn.accessToken.length > 0, 'Login did not return an access token.');
  assert(loggedIn.refreshToken !== undefined, 'Login did not return a refresh token.');
  assert(statuses.includes('authorized'), 'Login did not reach the authorized state.');
  await checkLoginForms(page, baseUrl);

  const refreshed = await auth.accessToken({ forceRefresh: true });
  assert(refreshed.length > 0, 'Refresh did not return an access token.');
  assert(await auth.logout(), 'Logout did not remove the cached profile.');
  process.stdout.write(`Keycloak ${KEYCLOAK_IMAGE} device-flow conformance passed.\n`);
} finally {
  await browser?.close();
  await run('docker', ['rm', '--force', containerName]).catch((error) => {
    if (
      !(error instanceof Error) ||
      !error.message.includes(`No such container: ${containerName}`)
    ) {
      throw error;
    }
  });
  await rm(temporaryDirectory, { recursive: true, force: true });
}

async function waitUntilReady(baseUrl) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(`${baseUrl}/realms/master/.well-known/openid-configuration`);
      if (response.ok) return;
    } catch {
      // Startup can refuse connections until the listener is ready.
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error('Keycloak did not become ready within 60 seconds.');
}

async function configureRealm(baseUrl) {
  const tokenResponse = await fetch(`${baseUrl}/realms/master/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: 'admin-cli',
      grant_type: 'password',
      username: 'admin',
      password: 'admin',
    }),
  });
  assert(tokenResponse.ok, 'Could not authenticate to the Keycloak admin API.');
  const tokenBody = await tokenResponse.json();
  assert(
    typeof tokenBody === 'object' &&
      tokenBody !== null &&
      'access_token' in tokenBody &&
      typeof tokenBody.access_token === 'string',
    'Keycloak admin API returned an invalid token response.',
  );
  const createResponse = await fetch(`${baseUrl}/admin/realms`, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${tokenBody.access_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      realm: REALM,
      enabled: true,
      sslRequired: 'none',
      registrationAllowed: false,
      oauth2DeviceCodeLifespan: 600,
      oauth2DevicePollingInterval: 1,
      users: [
        {
          username: USERNAME,
          email: 'conformance@example.test',
          emailVerified: true,
          enabled: true,
          firstName: 'Conformance',
          lastName: 'User',
          realmRoles: ['offline_access'],
          credentials: [{ type: 'password', value: PASSWORD, temporary: false }],
        },
      ],
      clients: [
        {
          clientId: CLIENT_ID,
          name: 'Baukit auth-node conformance',
          enabled: true,
          publicClient: true,
          standardFlowEnabled: true,
          redirectUris: [`${baseUrl}/callback`],
          directAccessGrantsEnabled: false,
          protocol: 'openid-connect',
          attributes: {
            'oauth2.device.authorization.grant.enabled': 'true',
            'pkce.code.challenge.method': 'S256',
          },
        },
      ],
    }),
  });
  assert(createResponse.status === 201, 'Could not create the Keycloak conformance realm.');
}

async function approveInBrowser(page, verificationUriComplete) {
  await page.goto(verificationUriComplete);
  await page.locator('#password').waitFor({ state: 'visible' });
  assert(await page.locator('#username').isVisible(), 'Fresh login has no username field.');
  await signInWithKeycloak(
    page,
    { username: USERNAME, password: PASSWORD },
    keycloakStack({
      url: new URL(verificationUriComplete).origin,
      realm: REALM,
      webClientId: CLIENT_ID,
    }),
  );
  await page.waitForLoadState('networkidle');
  const consent = page.getByRole('button', { name: 'Yes' });
  if (await consent.isVisible()) {
    await consent.click();
    await page.waitForLoadState('networkidle');
  }
  const text = await page.locator('body').innerText();
  assert(/success|connected|device/i.test(text), 'Keycloak did not confirm device approval.');
}

async function checkLoginForms(page, baseUrl) {
  await page.context().clearCookies();
  await page.route(`${baseUrl}/callback**`, (route) =>
    route.fulfill({ status: 200, body: 'Signed in.' }),
  );
  for (const usernameVisible of [true, false]) {
    const verifier = randomBytes(32).toString('base64url');
    const authorization = new URL(`${baseUrl}/realms/${REALM}/protocol/openid-connect/auth`);
    authorization.search = new URLSearchParams({
      client_id: CLIENT_ID,
      redirect_uri: `${baseUrl}/callback`,
      response_type: 'code',
      scope: 'openid',
      prompt: 'login',
      code_challenge_method: 'S256',
      code_challenge: createHash('sha256').update(verifier).digest('base64url'),
    }).toString();
    await page.goto(authorization.toString());
    await page.locator('#password').waitFor({ state: 'visible' });
    assert(
      (await page.locator('#username').isVisible()) === usernameVisible,
      usernameVisible
        ? 'Fresh login has no username field.'
        : 'SSO reauthentication has a visible username field.',
    );
    await signInWithKeycloak(
      page,
      { username: USERNAME, password: PASSWORD },
      keycloakStack({ url: baseUrl, realm: REALM, webClientId: CLIENT_ID }),
    );
    await page.waitForURL((url) => url.origin === baseUrl && url.pathname === '/callback');
    assert(page.url().includes('code='), 'Login did not return an authorization code.');
  }
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
