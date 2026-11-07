import { describe, expect, it } from 'vitest';

import {
  OidcClient,
  OidcError,
  buildAuthorizationUrl,
  normalizeIssuer,
  safeAuthErrorMessage,
} from './index.js';
import {
  discoveryResponse,
  fakeBrowser,
  jsonResponse,
  readLoginTransaction,
  required,
  type Tab,
} from './test-support/browser-session.js';

const CONFIG = {
  issuer: 'https://identity.example.test/realm',
  clientId: 'product-web',
  redirectUri: 'https://app.example.test/callback',
  storageKeyPrefix: 'oidc',
};

function setup(fetch: typeof globalThis.fetch = () => Promise.resolve(discoveryResponse())) {
  const browser = fakeBrowser(fetch);
  const tab = browser.tab({ coordinated: false });
  return { browser, tab, client: new OidcClient(CONFIG, tab.environment) };
}

async function loginState(client: OidcClient, tab: Tab): Promise<string> {
  await client.login();
  return required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
}

describe('OidcClient', () => {
  it('redirects with S256 PKCE and returns the verified code for the backend', async () => {
    const { tab, client } = setup();
    const state = await loginState(client, tab);
    const authorization = new URL(required(tab.navigations.at(-1)));
    expect(authorization.origin + authorization.pathname).toBe(
      'https://identity.example.test/realm/authorize',
    );
    expect(authorization.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authorization.searchParams.has('prompt')).toBe(false);
    const { verifier } = readLoginTransaction(tab.sessionStorage, 'oidc:transaction');

    tab.setCurrentUrl(
      `https://app.example.test/callback?code=one-time&state=${state}&iss=${encodeURIComponent(CONFIG.issuer)}`,
    );

    expect(client.completeCallback()).toEqual({
      code: 'one-time',
      codeVerifier: verifier,
      redirectUri: CONFIG.redirectUri,
    });
    expect(tab.sessionStorage.length).toBe(0);
  });

  it('ignores a URL without callback parameters', () => {
    const { client } = setup();
    expect(client.completeCallback()).toBeUndefined();
  });

  it.each([
    {
      name: 'a wrong state',
      query: (state: string) => `code=c&state=${state}x`,
      code: 'callback_state_mismatch',
    },
    {
      name: 'a missing code',
      query: (state: string) => `state=${state}`,
      code: 'callback_state_mismatch',
    },
    {
      name: 'a foreign issuer',
      query: (state: string) => `code=c&state=${state}&iss=https://evil.example.test`,
      code: 'issuer_mismatch',
    },
    {
      name: 'a provider error',
      query: (state: string) => `error=access_denied&state=${state}`,
      code: 'authorization_failed',
    },
  ])('rejects $name and consumes the transaction', async ({ query, code }) => {
    const { tab, client } = setup();
    const state = await loginState(client, tab);
    tab.setCurrentUrl(`https://app.example.test/callback?${query(state)}`);

    expect(() => client.completeCallback()).toThrow(expect.objectContaining({ code }));
    expect(tab.sessionStorage.length).toBe(0);
  });

  it('requests prompt=login until a fresh sign-in is confirmed', async () => {
    const { browser, tab, client } = setup();
    client.requireFreshLogin();
    await loginState(client, tab);
    expect(new URL(required(tab.navigations.at(-1))).searchParams.get('prompt')).toBe('login');
    expect(browser.localStorage.getItem('oidc:force-login')).toBe('1');

    client.confirmFreshLogin();
    await loginState(client, tab);
    expect(new URL(required(tab.navigations.at(-1))).searchParams.has('prompt')).toBe(false);
  });

  it('never touches a token storage key', async () => {
    const { browser, tab, client } = setup();
    const state = await loginState(client, tab);
    tab.setCurrentUrl(`https://app.example.test/callback?code=c&state=${state}`);
    client.completeCallback();
    expect(browser.localStorage.getItem('oidc:tokens')).toBeNull();
    expect(browser.localStorage.length).toBe(0);
  });

  it('uses only discovered endpoints and makes offline access opt-in', () => {
    const url = buildAuthorizationUrl(
      {
        clientId: 'client',
        redirectUri: 'https://app.example.test/callback',
        scopes: ['custom'],
      },
      'https://provider.example.test/non-keycloak/authorize',
      { state: 'state', challenge: 'challenge' },
    );
    expect(url.pathname).toBe('/non-keycloak/authorize');
    expect(url.searchParams.get('scope')).toBe('openid custom');
    expect(normalizeIssuer('https://provider.example.test/path///')).toBe(
      'https://provider.example.test/path',
    );
  });

  it('sanitizes provider errors and unknown errors', async () => {
    const leakedDescription = 'private content access-token-secret';
    const { tab, client } = setup();
    const state = await loginState(client, tab);
    tab.setCurrentUrl(
      `https://app.example.test/callback?error=access_denied&error_description=${encodeURIComponent(leakedDescription)}&state=${state}`,
    );

    let caught: unknown;
    try {
      client.completeCallback();
    } catch (cause) {
      caught = cause;
    }
    expect(caught).toBeInstanceOf(OidcError);
    expect(safeAuthErrorMessage(caught)).toBe('OIDC authorization failed.');
    expect(safeAuthErrorMessage(new Error(leakedDescription))).toBe('OIDC login failed.');
  });

  it('rejects mismatched discovery issuers without exposing metadata content', async () => {
    const { client } = setup(() =>
      Promise.resolve(
        jsonResponse({
          issuer: 'https://attacker.example.test/token-secret',
          authorization_endpoint: 'https://attacker.example.test/authorize',
          token_endpoint: 'https://attacker.example.test/token',
        }),
      ),
    );
    const failure = await client.discover().catch((error: unknown) => error);
    expect(failure).toMatchObject({ code: 'issuer_mismatch' });
    expect(safeAuthErrorMessage(failure)).not.toContain('token-secret');
  });
});
