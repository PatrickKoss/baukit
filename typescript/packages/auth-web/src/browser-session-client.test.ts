import { createApiRuntime } from '@baukit/api-runtime';
import { describe, expect, it, vi } from 'vitest';

import {
  BROWSER_SESSION_HEADER,
  BrowserSessionClient,
  OidcError,
  type SessionExpiredEvent,
} from './index.js';
import {
  jsonResponse,
  problem,
  readLoginTransaction,
  requestUrl,
  required,
} from './test-support/browser-session.js';
import {
  ACCESS_TOKEN,
  API,
  backend,
  CONFIG,
  LIFETIME_MS,
  ROTATED_TOKEN,
  sessionCalls,
  setup,
  signIn,
  storageEntries,
} from './test-support/session-backend.js';

describe('BrowserSessionClient', () => {
  it('redeems the code at the backend and keeps only an account hint in storage', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);

    await client.login();
    const transaction = readLoginTransaction(tab.sessionStorage, 'product:transaction');
    const rawTransaction: unknown = JSON.parse(
      required(tab.sessionStorage.getItem('product:transaction')),
    );
    expect(rawTransaction).toEqual(transaction);
    const authorization = new URL(required(tab.navigations.at(-1)));
    tab.setCurrentUrl(
      `https://app.example.test/?code=one-time-code&state=${required(authorization.searchParams.get('state'))}`,
    );
    const [first, second] = await Promise.all([client.handleCallback(), client.handleCallback()]);

    expect([first, second]).toEqual([true, true]);
    const exchanges = sessionCalls(server, 'exchange');
    expect(exchanges).toHaveLength(1);
    const init = required(exchanges[0]?.init);
    expect(init).toMatchObject({ method: 'POST', credentials: 'include', cache: 'no-store' });
    expect(new Headers(init.headers).get(BROWSER_SESSION_HEADER)).toBe('1');
    expect(JSON.parse(typeof init.body === 'string' ? init.body : '')).toEqual({
      code: 'one-time-code',
      codeVerifier: transaction.verifier,
      redirectUri: 'https://app.example.test/',
    });
    expect(tab.sessionStorage.length).toBe(0);
    expect(
      storageEntries(browser.localStorage)
        .map(([key]) => key)
        .sort(),
    ).toEqual(['product:account', 'product:refresh-generation']);
    expect(browser.localStorage.getItem('product:refresh-generation')).toBe('1');
    expect(JSON.parse(required(browser.localStorage.getItem('product:account')))).toEqual({
      subject: 'user-1',
      signedInAt: 1_000_000,
    });
    expect(client.account()).toEqual({ subject: 'user-1', signedInAt: 1_000_000 });
    await expect(client.getAccessToken()).resolves.toBe(ACCESS_TOKEN);
    expect(sessionCalls(server, 'refresh')).toHaveLength(0);
  });

  it('refreshes once on reload when the account hint exists and not at all without it', async () => {
    const { browser, server } = setup();
    const anonymous = new BrowserSessionClient(CONFIG, browser.tab().environment);
    expect(anonymous.hasSession()).toBe(false);
    await expect(anonymous.restore()).resolves.toBe(false);
    expect(server.calls).toHaveLength(0);

    await signIn(browser.tab());
    const reloaded = new BrowserSessionClient(CONFIG, browser.tab().environment);
    expect(reloaded.hasSession()).toBe(true);
    const tokens = await Promise.all([reloaded.restore(), reloaded.getAccessToken()]);

    expect(tokens).toEqual([true, ROTATED_TOKEN]);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    expect(required(sessionCalls(server, 'refresh')[0]?.init)).toMatchObject({
      method: 'POST',
      credentials: 'include',
    });
  });

  it.each([
    { lifetimeMs: 10_000, leeway: undefined, refreshAt: 1_005_000 },
    { lifetimeMs: 3_600_000, leeway: undefined, refreshAt: 4_570_000 },
    { lifetimeMs: 10_000, leeway: 0, refreshAt: 1_010_000 },
    { lifetimeMs: 10_000, leeway: 2, refreshAt: 1_008_000 },
  ])(
    'refreshes a $lifetimeMs ms token with leeway $leeway at $refreshAt',
    async ({ lifetimeMs, leeway, refreshAt }) => {
      const { browser, server } = setup(lifetimeMs);
      const tab = browser.tab();
      const client = await signIn(
        tab,
        new BrowserSessionClient(
          { ...CONFIG, ...(leeway === undefined ? {} : { refreshLeewaySeconds: leeway }) },
          tab.environment,
        ),
      );
      for (const at of [1_000_000, refreshAt - 1]) {
        browser.setNow(at);
        await expect(client.getAccessToken()).resolves.toBe(ACCESS_TOKEN);
      }
      expect(sessionCalls(server, 'refresh')).toHaveLength(0);
      browser.setNow(refreshAt);
      await expect(client.getAccessToken()).resolves.toBe(ROTATED_TOKEN);
      expect(sessionCalls(server, 'refresh')).toHaveLength(1);
      await expect(client.getAccessToken({ forceRefresh: true })).resolves.toBe(ROTATED_TOKEN);
      expect(sessionCalls(server, 'refresh')).toHaveLength(2);
    },
  );

  it('runs one refresh across three tabs and hands the token to the waiting tabs', async () => {
    const { browser, server } = setup();
    await signIn(browser.tab());
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const respond = server.refresh;
    server.refresh = async () => {
      await gate;
      return respond();
    };
    const tabs = [browser.tab(), browser.tab(), browser.tab()].map(
      (tab) => new BrowserSessionClient(CONFIG, tab.environment),
    );

    const pending = Promise.all(tabs.map((client) => client.getAccessToken()));
    await vi.waitFor(() => {
      expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    });
    required(release)();

    await expect(pending).resolves.toEqual([ROTATED_TOKEN, ROTATED_TOKEN, ROTATED_TOKEN]);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    expect(browser.locks.requests).toEqual([
      'product:refresh',
      'product:refresh',
      'product:refresh',
    ]);
    for (const client of tabs) {
      await expect(client.getAccessToken()).resolves.toBe(ROTATED_TOKEN);
    }
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
  });

  it('signs out on 401 in every tab and clears the account hint', async () => {
    const { browser, server } = setup();
    const first = await signIn(browser.tab());
    const second = new BrowserSessionClient(CONFIG, browser.tab().environment);
    await second.getAccessToken();
    const events: SessionExpiredEvent[] = [];
    first.onSessionExpired((event) => events.push(event));
    second.subscribeSessionExpired((event) => events.push(event));
    server.refresh = () => problem(401, 'session_expired');

    await expect(first.getAccessToken({ forceRefresh: true })).resolves.toBeUndefined();
    await vi.waitFor(() => {
      expect(events).toHaveLength(2);
    });

    expect(events).toEqual([
      { type: 'session-expired', reason: 'refresh_rejected' },
      { type: 'session-expired', reason: 'refresh_rejected' },
    ]);
    expect(browser.localStorage.getItem('product:account')).toBeNull();
    expect(first.hasSession()).toBe(false);
    expect(second.hasSession()).toBe(false);
    await expect(second.getAccessToken()).resolves.toBeUndefined();
  });

  it('lets waiting tabs observe a 401 once instead of refreshing again', async () => {
    const { browser, server } = setup();
    await signIn(browser.tab());
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.refresh = async () => {
      await gate;
      return problem(401, 'session_expired');
    };
    const tabs = [browser.tab(), browser.tab(), browser.tab()].map(
      (tab) => new BrowserSessionClient(CONFIG, tab.environment),
    );
    const events = tabs.map(() => [] as SessionExpiredEvent[]);
    tabs.forEach((client, index) => {
      client.onSessionExpired((event) => required(events[index]).push(event));
    });

    const pending = Promise.all(tabs.map((client) => client.getAccessToken()));
    await vi.waitFor(() => {
      expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    });
    required(release)();

    await expect(pending).resolves.toEqual([undefined, undefined, undefined]);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    expect(events.map((list) => list.length)).toEqual([1, 1, 1]);
  });

  it.each([
    {
      name: 'a network error',
      response: () => Promise.reject(new TypeError('offline')),
      status: undefined,
      retryable: true,
    },
    {
      name: 'a rate limit',
      response: () => problem(429, 'rate_limited'),
      status: 429,
      retryable: true,
    },
    {
      name: 'an unavailable IdP',
      response: () => problem(503, 'idp_unavailable'),
      status: 503,
      retryable: true,
    },
    {
      name: 'a CSRF rejection',
      response: () => problem(403, 'csrf_rejected'),
      status: 403,
      retryable: false,
    },
  ])('keeps the account hint after $name', async ({ response, status, retryable }) => {
    const { browser, server } = setup();
    await signIn(browser.tab());
    const offline = new BrowserSessionClient(CONFIG, browser.tab().environment);
    const expired = vi.fn();
    offline.onSessionExpired(expired);
    server.refresh = response;

    const failure = await offline.getAccessToken().catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(OidcError);
    expect(failure).toMatchObject({ code: 'refresh_failed', status, retryable });
    expect(offline.hasSession()).toBe(true);
    expect(offline.account()?.subject).toBe('user-1');
    expect(expired).not.toHaveBeenCalled();
    server.refresh = backend(() => 1_000_000).refresh;
    await expect(offline.getAccessToken()).resolves.toBe(ROTATED_TOKEN);
  });

  it('updates the account hint when the refreshed subject changes', async () => {
    const { browser, server, now } = setup();
    const client = await signIn(browser.tab());
    browser.setNow(2_000_000);
    server.refresh = () =>
      jsonResponse({
        accessToken: ROTATED_TOKEN,
        expiresAt: now() + LIFETIME_MS,
        subject: 'user-2',
      });

    await client.getAccessToken({ forceRefresh: true });

    expect(client.account()).toEqual({ subject: 'user-2', signedInAt: 2_000_000 });
  });

  it('logs out at the backend, signs out other tabs and follows only an http(s) end-session URL', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = await signIn(tab);
    const other = new BrowserSessionClient(CONFIG, browser.tab().environment);
    await other.getAccessToken();
    const otherEvents: SessionExpiredEvent[] = [];
    other.onSessionExpired((event) => otherEvents.push(event));
    const ownEvents = vi.fn();
    client.onSessionExpired(ownEvents);
    server.logout = () =>
      jsonResponse({ endSessionUrl: 'https://identity.example.test/realm/logout?id_token_hint=x' });

    await expect(client.logout()).resolves.toBe(true);
    await vi.waitFor(() => {
      expect(otherEvents).toEqual([{ type: 'session-expired', reason: 'signed_out' }]);
    });

    expect(required(sessionCalls(server, 'logout')[0]?.init)).toMatchObject({
      method: 'POST',
      credentials: 'include',
    });
    expect(tab.navigations.at(-1)).toBe(
      'https://identity.example.test/realm/logout?id_token_hint=x',
    );
    expect(ownEvents).not.toHaveBeenCalled();
    expect(client.hasSession()).toBe(false);
    expect(other.hasSession()).toBe(false);

    server.logout = () => jsonResponse({ endSessionUrl: 'javascript:alert(1)' });
    await expect(client.logout()).resolves.toBe(false);
    server.logout = () => new Response(null, { status: 204 });
    await expect(client.logout()).resolves.toBe(false);
    server.logout = () => problem(503, 'idp_unavailable');
    await expect(client.logout()).rejects.toMatchObject({ code: 'logout_failed', status: 503 });
    expect(tab.navigations.at(-1)).toBe(
      'https://identity.example.test/realm/logout?id_token_hint=x',
    );
  });

  it('requires fresh authentication after erasure until a successful exchange', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = await signIn(tab);
    const expired = vi.fn();
    client.onSessionExpired(expired);

    client.clearSession({ reason: 'profile_erased' });

    expect(expired).toHaveBeenCalledWith({ type: 'session-expired', reason: 'profile_erased' });
    expect(browser.localStorage.getItem('product:force-login')).toBe('1');
    expect(client.hasSession()).toBe(false);
    await client.login();
    const authorization = new URL(required(tab.navigations.at(-1)));
    expect(authorization.searchParams.get('prompt')).toBe('login');
    const fresh = new BrowserSessionClient(CONFIG, tab.environment);
    tab.setCurrentUrl(
      `https://app.example.test/?code=fresh&state=${required(authorization.searchParams.get('state'))}`,
    );
    await expect(fresh.handleCallback()).resolves.toBe(true);
    expect(browser.localStorage.getItem('product:force-login')).toBeNull();
    expect(sessionCalls(server, 'exchange')).toHaveLength(2);
  });

  it('does not restore a session cleared while the exchange is in flight', async () => {
    const { browser, server } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);
    await client.login();
    const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
    tab.setCurrentUrl(`https://app.example.test/?code=c&state=${state}`);
    let complete: ((response: Response) => void) | undefined;
    browser.fetch.mockImplementation((input, init) =>
      requestUrl(input).endsWith('/exchange')
        ? new Promise((resolve) => {
            complete = resolve;
          })
        : server.fetch(input, init),
    );
    const callback = client.handleCallback();
    await vi.waitFor(() => {
      expect(complete).toBeDefined();
    });

    client.clearSession();
    required(complete)(
      jsonResponse({ accessToken: ACCESS_TOKEN, expiresAt: 2_000_000, subject: 's' }),
    );

    await expect(callback).resolves.toBe(true);
    expect(client.hasSession()).toBe(false);
    expect(browser.localStorage.getItem('product:account')).toBeNull();
  });

  it('rejects an exchange the backend refuses without leaking its body', async () => {
    const { browser } = setup();
    const tab = browser.tab();
    const client = new BrowserSessionClient(CONFIG, tab.environment);
    await client.login();
    const state = required(new URL(required(tab.navigations.at(-1))).searchParams.get('state'));
    tab.setCurrentUrl(`https://app.example.test/?code=one-time-code&state=${state}`);
    browser.fetch.mockResolvedValue(new Response('private-content one-time-code', { status: 400 }));

    const failure = await client.handleCallback().catch((error: unknown) => error);

    expect(failure).toMatchObject({ code: 'token_exchange_failed', status: 400 });
    expect(String(failure)).not.toContain('one-time-code');
    expect(client.hasSession()).toBe(false);
  });

  it('works without Web Locks or BroadcastChannel', async () => {
    const { browser, server } = setup();
    await signIn(browser.tab({ coordinated: false }));
    const client = new BrowserSessionClient(
      CONFIG,
      browser.tab({ coordinated: false }).environment,
    );

    await expect(client.getAccessToken()).resolves.toBe(ROTATED_TOKEN);
    expect(browser.locks.requests).toEqual([]);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
  });

  it('sends cookies only on session calls; API calls keep same-origin credentials', async () => {
    const { browser, server } = setup();
    const client = await signIn(browser.tab());
    await client.getAccessToken({ forceRefresh: true });
    const apiRequests: Request[] = [];
    const api = createApiRuntime({
      baseUrl: API,
      environment: 'test',
      fetch: (input, init) => {
        apiRequests.push(new Request(input, init));
        return Promise.resolve(jsonResponse({ ok: true }));
      },
      tokenProvider: async () => (await client.getAccessToken()) ?? null,
    });
    await api.fetch(`${API}/api/v1/me`);
    await client.logout();

    for (const { url, init } of server.calls) {
      const isSession = url.startsWith(`${API}/auth/session/`);
      expect(init?.credentials === 'include', url).toBe(isSession);
    }
    expect(sessionCalls(server, 'exchange')).toHaveLength(1);
    expect(sessionCalls(server, 'refresh')).toHaveLength(1);
    expect(sessionCalls(server, 'logout')).toHaveLength(1);
    const apiRequest = required(apiRequests[0]);
    expect(apiRequest.credentials).toBe('same-origin');
    expect(apiRequest.headers.get('authorization')).toBe(`Bearer ${ROTATED_TOKEN}`);
  });
});
