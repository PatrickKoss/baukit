import { execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';

import { chromium } from '@playwright/test';

import { DeviceFlowClient } from '../dist/index.js';

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
  const port = await availablePort();
  const baseUrl = `http://127.0.0.1:${String(port)}`;
  await run('docker', [
    'run',
    '--detach',
    '--rm',
    '--name',
    containerName,
    '--publish',
    `127.0.0.1:${String(port)}:8080`,
    '--env',
    'KC_BOOTSTRAP_ADMIN_USERNAME=admin',
    '--env',
    'KC_BOOTSTRAP_ADMIN_PASSWORD=admin',
    KEYCLOAK_IMAGE,
    'start-dev',
  ]);
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

  const refreshed = await auth.accessToken({ forceRefresh: true });
  assert(refreshed.length > 0, 'Refresh did not return an access token.');
  assert(await auth.logout(), 'Logout did not remove the cached profile.');
  process.stdout.write(`Keycloak ${KEYCLOAK_IMAGE} device-flow conformance passed.\n`);
} finally {
  await browser?.close();
  await run('docker', ['rm', '--force', containerName]).catch(() => undefined);
  await rm(temporaryDirectory, { recursive: true, force: true });
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (address === null || typeof address === 'string') {
    server.close();
    throw new Error('Could not allocate a Keycloak port.');
  }
  await new Promise((resolve, reject) => {
    server.close((error) => {
      if (error === undefined) resolve();
      else reject(error);
    });
  });
  return address.port;
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
          standardFlowEnabled: false,
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
  await page.locator('#username').fill(USERNAME);
  await page.locator('#password').fill(PASSWORD);
  await page.getByRole('button', { name: 'Sign In' }).click();
  await page.waitForLoadState('networkidle');
  const consent = page.getByRole('button', { name: 'Yes' });
  if (await consent.isVisible()) {
    await consent.click();
    await page.waitForLoadState('networkidle');
  }
  const text = await page.locator('body').innerText();
  assert(/success|connected|device/i.test(text), 'Keycloak did not confirm device approval.');
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
