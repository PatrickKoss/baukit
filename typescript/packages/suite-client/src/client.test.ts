import { describe, expect, it, vi } from 'vitest';
import { compareCatalogKeys } from '@baukit/localization-core';
import { type OAuthInFlightSession, type OAuthSessionResult } from '@baukit/integrations-client';
import {
  SuiteClient,
  SuiteFlowError,
  SuiteSession,
  SuiteAuthorizeMachine,
  SuiteLinkedMachine,
  ConnectedApps,
  suiteReturnValidator,
  createSuiteNativeIntentValidator,
  openSuitePeer,
  suiteConnectionState,
  parseSuiteAuthorizeQuery,
  switchSuiteAccount,
  createSuiteNavigationStore,
  suiteMessages,
  type SuiteLink,
  type SuiteTransport,
  type AuthorizationPreview,
} from './index.js';

const link: SuiteLink = {
  id: 'link',
  peerApp: 'beta',
  role: 'initiator',
  remoteLinkId: 'remote',
  remoteDisplayName: 'User',
  status: 'active',
  sends: ['alpha.activity.completed'],
  receives: ['beta.activity.completed'],
  shareXp: true,
  rewardMode: 'native',
  deliveryHealth: 'healthy',
  consecutiveFailures: 0,
  lastDeliveryAt: null,
  lastFailureAt: null,
  lastFailureCode: null,
  lastReceivedAt: null,
  replayEarliest: '2025-10-07',
  createdAt: '2026-10-07T00:00:00Z',
  updatedAt: '2026-10-07T00:00:00Z',
};
const preview: AuthorizationPreview = {
  peer: 'Beta',
  authorizerSends: ['beta.activity.completed'],
  initiatorSends: ['alpha.activity.completed'],
  existingLink: false,
  autoApprove: false,
  hintMismatch: false,
};
const input = {
  client: 'alpha',
  state: 'state',
  codeChallenge: 'challenge',
  hint: 'hint',
  hintDomain: 'shared',
};
function apiFixture() {
  const request = vi.fn<(method: string, path: string, body?: unknown) => Promise<unknown>>(
    (_method, path) => {
      if (path.includes('/preview?')) return Promise.resolve(preview);
      if (path === '/suite/links' && _method === 'POST')
        return Promise.resolve({
          requestId: 'request',
          authorizeUrl: 'https://beta.example/suite/authorize',
        });
      if (path === '/suite/peers' || (path === '/suite/links' && _method === 'GET'))
        return Promise.resolve([]);
      if (path.endsWith('/complete')) return Promise.resolve(link);
      return Promise.resolve({ redirectUrl: 'https://alpha.example/suite/links/callback' });
    },
  );
  const transport: SuiteTransport = {
    async request<T>(
      method: 'GET' | 'POST' | 'PATCH' | 'DELETE',
      path: string,
      body?: unknown,
    ): Promise<T> {
      return (await request(method, path, body)) as T;
    },
  };
  return { client: new SuiteClient(transport), request };
}
function sessionFixture(platform: 'web' | 'native' = 'native') {
  const api = apiFixture();
  let stored: OAuthInFlightSession | null = null;
  const storage = {
    load: () => Promise.resolve(stored),
    save: (value: OAuthInFlightSession) => {
      stored = value;
      return Promise.resolve();
    },
    clear: () => {
      stored = null;
      return Promise.resolve();
    },
  };
  const callback =
    platform === 'web'
      ? 'https://alpha.example/suite/linked?state=nonce&request=request&code=code'
      : 'alpha://suite/linked?state=nonce&request=request&code=code';
  const run = vi.fn<() => Promise<OAuthSessionResult>>(() =>
    Promise.resolve({ type: 'success', returnUrl: callback }),
  );
  const cancel = vi.fn();
  const timers = new Map<number, () => void>();
  let next = 0;
  const session = new SuiteSession({
    client: api.client,
    platform,
    origin: platform === 'web' ? 'https://alpha.example' : 'alpha://suite',
    coordinator: {
      storage,
      createStateNonce: () => 'nonce',
      clock: {
        now: () => 100,
        setTimeout: (callback) => {
          timers.set(++next, callback);
          return next;
        },
        clearTimeout: (handle) => {
          if (typeof handle === 'number') timers.delete(handle);
        },
      },
      native: { run, cancel },
      redirect: { authorize: run },
    },
  });
  return { ...api, session, storage, callback, run, cancel, timers };
}

describe('suite API', () => {
  it('uses all authenticated route shapes and encodes path and query values', async () => {
    const { client, request } = apiFixture();
    await client.peers();
    await client.list();
    await client.get('a/b');
    await client.preview('a&b', 'x+y', 'd e');
    await client.authorize(input);
    await client.deny({ client: 'alpha', state: 'state' });
    await client.update('a/b', { shareXp: false, rewardMode: 'off' });
    await client.disconnect('a/b');
    await client.test('a/b');
    await client.reenable('a/b');
    await client.replay('a/b', '2026-01-01');
    await client.deliveries('a/b');
    expect(request.mock.calls).toEqual([
      ['GET', '/suite/peers', undefined],
      ['GET', '/suite/links', undefined],
      ['GET', '/suite/links/a%2Fb', undefined],
      ['GET', '/suite/authorizations/preview?client=a%26b&hint=x%2By&hint_domain=d+e', undefined],
      ['POST', '/suite/authorizations', input],
      ['POST', '/suite/authorizations/deny', { client: 'alpha', state: 'state' }],
      ['PATCH', '/suite/links/a%2Fb', { shareXp: false, rewardMode: 'off' }],
      ['DELETE', '/suite/links/a%2Fb', undefined],
      ['POST', '/suite/links/a%2Fb/test', undefined],
      ['POST', '/suite/links/a%2Fb/reenable', undefined],
      ['POST', '/suite/links/a%2Fb/replay', { since: '2026-01-01' }],
      ['GET', '/suite/links/a%2Fb/deliveries', undefined],
    ]);
  });
});
describe('return URLs and peer links', () => {
  it('validates exact web and native endpoints', () => {
    const web = suiteReturnValidator({ platform: 'web', origin: 'https://alpha.example' });
    const native = suiteReturnValidator({ platform: 'native', origin: 'alpha://suite' });
    expect(web('https://alpha.example/suite/linked?code=x')).toBe(true);
    expect(native('alpha://suite/linked?state=x')).toBe(true);
    for (const url of [
      'https://evil.example/suite/linked',
      'https://alpha.example.evil/suite/linked',
      'https://user@alpha.example/suite/linked',
      'https://alpha.example/suite/linked/extra',
      'http://alpha.example/suite/linked',
    ])
      expect(web(url)).toBe(false);
    for (const url of [
      'alpha://auth/callback',
      'alpha://suite/linked/extra',
      'alpha:///suite/linked',
      'beta://suite/linked',
      'alpha://suite.evil/linked',
    ])
      expect(native(url)).toBe(false);
  });
  it('allows only suite native pages and retains their queries', () => {
    const validate = createSuiteNativeIntentValidator('alpha-app');
    for (const url of [
      'alpha-app://suite/linked?code=x',
      'alpha-app:///suite/authorize?client=x',
      '/suite/linked',
    ])
      expect(validate(url)).toBe(true);
    for (const url of [
      'beta://suite/linked',
      '/suite/linked/extra',
      'alpha-app://suite/%6cinked',
      '/auth/callback',
    ])
      expect(validate(url)).toBe(false);
    expect(() => createSuiteNativeIntentValidator('javascript:')).toThrow(TypeError);
  });
  it('opens the configured web origin on web', async () => {
    const openUrl = vi.fn(() => Promise.resolve());
    await openSuitePeer(
      { scheme: 'beta', webUrl: 'https://beta.example' },
      { platform: 'web', openUrl },
    );
    expect(openUrl.mock.calls).toEqual([['https://beta.example']]);
  });
  it('opens the native app and falls back to web only on failure', async () => {
    const openUrl = vi
      .fn<(url: string) => Promise<void>>()
      .mockRejectedValueOnce(new Error('absent'))
      .mockResolvedValueOnce(undefined);
    await openSuitePeer(
      { scheme: 'beta.app+native', webUrl: 'https://beta.example' },
      { platform: 'native', openUrl },
    );
    expect(openUrl.mock.calls).toEqual([['beta.app+native://'], ['https://beta.example']]);
    openUrl.mockReset().mockResolvedValue(undefined);
    await openSuitePeer(
      { scheme: 'beta', webUrl: 'https://beta.example' },
      { platform: 'native', openUrl },
    );
    expect(openUrl).toHaveBeenCalledTimes(1);
  });
  it('propagates failure when neither native nor web can open', async () => {
    const openUrl = vi.fn(() => Promise.reject(new Error('unavailable')));
    await expect(
      openSuitePeer(
        { scheme: 'beta', webUrl: 'https://beta.example' },
        { platform: 'native', openUrl },
      ),
    ).rejects.toThrow('unavailable');
    expect(openUrl).toHaveBeenCalledTimes(2);
  });
  it('requires explicit loopback HTTP and rejects unsafe URLs on web too', async () => {
    const openUrl = vi.fn(() => Promise.resolve());
    for (const webUrl of [
      'javascript:alert(1)',
      'http://beta.example',
      'https://user:pass@beta.example',
    ])
      await expect(
        openSuitePeer({ scheme: 'beta', webUrl }, { platform: 'web', openUrl }),
      ).rejects.toThrow(TypeError);
    expect(openUrl).not.toHaveBeenCalled();
    await openSuitePeer(
      { scheme: 'beta', webUrl: 'http://localhost:1234' },
      { platform: 'web', openUrl, allowLoopback: true },
    );
    expect(openUrl).toHaveBeenCalledWith('http://localhost:1234');
  });
});
describe('OAuth session glue', () => {
  it('opens a shared native session and redeems the validated code once', async () => {
    const f = sessionFixture();
    const result = f.session.connect('beta');
    expect(f.session.handleRedirect(f.callback)).toBe(result);
    expect(await result).toEqual({ type: 'connected', requestId: 'request', link });
    expect(await f.session.handleRedirect(f.callback)).toEqual(await result);
    expect(f.request.mock.calls).toEqual([
      [
        'POST',
        '/suite/links',
        { peerApp: 'beta', returnUrl: 'alpha://suite/linked', stateNonce: 'nonce' },
      ],
      ['POST', '/suite/links/requests/request/complete', { code: 'code' }],
    ]);
    expect(f.run).toHaveBeenCalledTimes(1);
  });
  it('treats dismiss as cancellation', async () => {
    const f = sessionFixture();
    f.run.mockResolvedValue({ type: 'cancelled' });
    expect(await f.session.connect('beta')).toEqual({ type: 'cancelled' });
    expect(await f.storage.load()).toBeNull();
    expect(f.request).toHaveBeenCalledTimes(1);
  });
  it.each(['native', 'web'] as const)(
    'resumes a %s callback after a cold launch',
    async (platform) => {
      const f = sessionFixture(platform);
      await f.storage.save({
        stateNonce: 'nonce',
        returnUrl: f.callback.split('?')[0] ?? '',
        createdAt: 100,
      });
      expect(await f.session.handleRedirect(f.callback)).toEqual({
        type: 'connected',
        requestId: 'request',
        link,
      });
      expect(f.run).not.toHaveBeenCalled();
    },
  );
  it('uses the current web tab adapter and persists state before navigation', async () => {
    const f = sessionFixture('web');
    f.run.mockImplementation(async () => {
      expect((await f.storage.load())?.stateNonce).toBe('nonce');
      return { type: 'success', returnUrl: f.callback };
    });
    expect((await f.session.connect('beta')).type).toBe('connected');
    expect(f.run).toHaveBeenCalledTimes(1);
  });
  it.each([
    'status=denied',
    'status=failed&code=suite_peer_unreachable',
    'status=other',
    'request=request',
    'request=request&code=code&state=duplicate',
  ])('handles %s without redeeming an untrusted code', async (query) => {
    const f = sessionFixture();
    f.run.mockResolvedValue({
      type: 'success',
      returnUrl: `alpha://suite/linked?state=nonce&${query}`,
    });
    if (query === 'status=denied')
      expect(await f.session.connect('beta')).toEqual({ type: 'denied' });
    else await expect(f.session.connect('beta')).rejects.toBeInstanceOf(SuiteFlowError);
    expect(f.request).toHaveBeenCalledTimes(1);
  });
  it('rejects a return with the wrong nonce or origin', async () => {
    for (const returnUrl of [
      'alpha://suite/linked?state=wrong&request=r&code=c',
      'beta://suite/linked?state=nonce&request=r&code=c',
    ]) {
      const f = sessionFixture();
      f.run.mockResolvedValue({ type: 'success', returnUrl });
      await expect(f.session.connect('beta')).rejects.toMatchObject({ code: 'suite_code_invalid' });
      expect(f.request).toHaveBeenCalledTimes(1);
    }
  });
});
describe('authorize state', () => {
  it('previews both directions with the identity domain then posts consent', async () => {
    const f = apiFixture();
    const machine = new SuiteAuthorizeMachine(f.client, input, {
      web: true,
      framed: false,
      signedIn: true,
    });
    expect(await machine.load()).toEqual({ type: 'consent', preview });
    expect(await machine.approve()).toEqual({
      type: 'redirect',
      url: 'https://alpha.example/suite/links/callback',
    });
    expect(f.request.mock.calls[0]).toEqual([
      'GET',
      '/suite/authorizations/preview?client=alpha&hint=hint&hint_domain=shared',
      undefined,
    ]);
    expect(f.request.mock.calls[1]).toEqual([
      'POST',
      '/suite/authorizations',
      { client: 'alpha', state: 'state', codeChallenge: 'challenge', hint: 'hint' },
    ]);
  });
  it('auto-approves once', async () => {
    const f = apiFixture();
    f.request.mockResolvedValueOnce({ ...preview, autoApprove: true });
    const machine = new SuiteAuthorizeMachine(f.client, input, {
      web: true,
      framed: false,
      signedIn: true,
    });
    expect((await machine.load()).type).toBe('redirect');
    await machine.load();
    await machine.approve();
    expect(f.request).toHaveBeenCalledTimes(2);
  });
  it('denies consent and follows the API redirect', async () => {
    const f = apiFixture();
    const machine = new SuiteAuthorizeMachine(f.client, input, {
      web: true,
      framed: false,
      signedIn: true,
    });
    await machine.load();
    expect((await machine.deny()).type).toBe('redirect');
    expect(f.request.mock.calls[1]).toEqual([
      'POST',
      '/suite/authorizations/deny',
      { client: 'alpha', state: 'state' },
    ]);
  });
  it('blocks consent and auto-approve on a hint mismatch', async () => {
    const f = apiFixture();
    f.request.mockResolvedValueOnce({ ...preview, autoApprove: true, hintMismatch: true });
    const machine = new SuiteAuthorizeMachine(f.client, input, {
      web: true,
      framed: false,
      signedIn: true,
    });
    expect((await machine.load()).type).toBe('hint_mismatch');
    await machine.approve();
    expect(f.request).toHaveBeenCalledTimes(1);
    expect((await machine.deny()).type).toBe('redirect');
  });
  it('waits for logout before starting account-switch login and retains the query', async () => {
    const order: string[] = [];
    const logout = vi.fn((path: string) => {
      order.push('logout');
      expect(new URL(path, 'https://alpha.example').searchParams.get('hint_domain')).toBe('shared');
      return Promise.resolve();
    });
    const login = vi.fn(() => {
      expect(order).toEqual(['logout']);
      order.push('login');
      return Promise.resolve();
    });
    await switchSuiteAccount(input, { logout, login });
    expect(order).toEqual(['logout', 'login']);
  });
  it.each([
    { web: true, framed: true, signedIn: true, type: 'framed' },
    { web: false, framed: false, signedIn: true, type: 'web_required' },
    { web: true, framed: false, signedIn: false, type: 'login' },
  ])('refuses authorization in $type state', async ({ type, ...environment }) => {
    const f = apiFixture();
    const machine = new SuiteAuthorizeMachine(f.client, input, environment);
    expect((await machine.load()).type).toBe(type);
    await machine.approve();
    await machine.deny();
    expect(f.request).not.toHaveBeenCalled();
  });
  it('rejects invalid and duplicate query values', () => {
    for (const query of [
      '',
      'client=a&state=s',
      'client=a&state=s&code_challenge=c&hint=a&hint=b',
      'client=a&client=b&state=s&code_challenge=c',
    ])
      expect(parseSuiteAuthorizeQuery(new URLSearchParams(query))).toBeNull();
    expect(
      parseSuiteAuthorizeQuery(
        new URLSearchParams(
          'client=alpha&state=state&code_challenge=challenge&hint=hint&hint_domain=shared',
        ),
      ),
    ).toEqual(input);
  });
  it('keeps consent available after coded authorization failure', async () => {
    const f = apiFixture();
    const machine = new SuiteAuthorizeMachine(f.client, input, {
      web: true,
      framed: false,
      signedIn: true,
    });
    await machine.load();
    f.request.mockRejectedValueOnce(new SuiteFlowError('suite_link_account_mismatch'));
    expect(await machine.approve()).toEqual({
      type: 'failed',
      code: 'suite_link_account_mismatch',
    });
    expect((await machine.approve()).type).toBe('redirect');
  });
});
describe('connected apps and linked page', () => {
  it.each([
    { order: 'native_first', refreshFails: false },
    { order: 'redirect_first', refreshFails: false },
    { order: 'native_first', refreshFails: true },
    { order: 'redirect_first', refreshFails: true },
  ] as const)(
    'returns the completed request and announces once when $order and refreshFails=$refreshFails',
    async ({ order, refreshFails }) => {
      const f = sessionFixture();
      const list = vi.spyOn(f.client, 'list');
      if (refreshFails) list.mockRejectedValue(new SuiteFlowError('suite_peer_unreachable'));
      const connected = new ConnectedApps(f.client, f.session);
      const navigation = createSuiteNavigationStore(() => 'notice');
      const linked = new SuiteLinkedMachine({
        originalUrl: f.callback,
        session: f.session,
        navigation,
        refresh: async () => {
          await connected.load();
        },
        scrubHistory: () => undefined,
      });
      const native = connected.connect('beta');
      const redirect = linked.restore(false, true);
      const claims: boolean[] = [];
      const claimNative = async () => {
        const state = await native;
        expect(state).toEqual({
          ...(refreshFails
            ? { type: 'failed', code: 'suite_peer_unreachable' }
            : { type: 'ready', peers: [], links: [] }),
          completed: { peerApp: 'beta', requestId: 'request' },
        });
        if ((state.type === 'ready' || state.type === 'failed') && state.completed) {
          claims.push(navigation.claimSuiteConnectionAnnouncement(state.completed.requestId));
        }
      };
      const claimRedirect = async () => {
        const state = await redirect;
        expect(state).toEqual({ type: 'connected_apps', noticeToken: 'notice' });
        if (state.type === 'connected_apps') {
          claims.push(navigation.consumeSuiteNotice(state.noticeToken)?.announce ?? false);
        }
      };
      if (order === 'native_first') {
        await claimNative();
        await claimRedirect();
      } else {
        await claimRedirect();
        await claimNative();
      }
      expect(claims).toEqual([true, false]);
      expect(f.request.mock.calls.filter(([, path]) => path.endsWith('/complete'))).toHaveLength(1);
      list.mockRestore();
      expect(await connected.load()).toEqual({ type: 'ready', peers: [], links: [] });
    },
  );

  it('waits for restoration, scrubs history, completes and refetches before navigating', async () => {
    const f = sessionFixture('web');
    await f.storage.save({
      stateNonce: 'nonce',
      returnUrl: 'https://alpha.example/suite/linked',
      createdAt: 100,
    });
    const refresh = vi.fn(() => Promise.resolve());
    const scrubHistory = vi.fn();
    const machine = new SuiteLinkedMachine({
      originalUrl: f.callback,
      session: f.session,
      navigation: createSuiteNavigationStore(() => 'notice'),
      refresh,
      scrubHistory,
    });
    expect(scrubHistory).toHaveBeenCalledWith('/suite/linked');
    expect(await machine.restore(true, true)).toEqual({ type: 'restoring' });
    expect(f.request).not.toHaveBeenCalled();
    expect(await machine.restore(false, true)).toEqual({
      type: 'connected_apps',
      noticeToken: 'notice',
    });
    await machine.restore(false, true);
    expect(f.request).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveBeenCalledTimes(1);
  });
  it('requires login before redeeming a return and retains its query', async () => {
    const f = sessionFixture('web');
    const machine = new SuiteLinkedMachine({
      originalUrl: f.callback,
      session: f.session,
      navigation: createSuiteNavigationStore(() => 'notice'),
      refresh: () => Promise.resolve(),
      scrubHistory: () => undefined,
    });
    expect(await machine.restore(false, false)).toEqual({
      type: 'login',
      returnPath: '/suite/linked?state=nonce&request=request&code=code',
    });
    expect(f.request).not.toHaveBeenCalled();
  });
  it('returns coded failure through a one-shot notice', async () => {
    const f = sessionFixture();
    const navigation = createSuiteNavigationStore(() => 'notice');
    const machine = new SuiteLinkedMachine({
      originalUrl: f.callback,
      session: f.session,
      navigation,
      refresh: () => Promise.resolve(),
      scrubHistory: () => undefined,
    });
    expect(await machine.restore(false, true)).toEqual({
      type: 'connected_apps',
      noticeToken: 'notice',
    });
    expect(navigation.consumeSuiteNotice('notice')?.notice.errorCode).toBe('suite_code_invalid');
  });
  it('loads peers and links and handles denied, failed and reconnect flows', async () => {
    const f = sessionFixture();
    const connected = new ConnectedApps(f.client, f.session);
    expect(await connected.load()).toEqual({ type: 'ready', peers: [], links: [] });
    f.run.mockResolvedValueOnce({
      type: 'success',
      returnUrl: 'alpha://suite/linked?state=nonce&status=denied',
    });
    expect(await connected.connect('beta')).toEqual({ type: 'denied' });
    f.run.mockResolvedValueOnce({
      type: 'success',
      returnUrl: 'alpha://suite/linked?state=nonce&status=failed&code=suite_peer_unreachable',
    });
    expect(await connected.connect('beta')).toEqual({
      type: 'failed',
      code: 'suite_peer_unreachable',
    });
    expect((await connected.connect('beta')).type).toBe('ready');
    expect(suiteConnectionState(undefined)).toBe('not_connected');
    expect(suiteConnectionState(link)).toBe('connected');
    expect(
      suiteConnectionState({ ...link, status: 'needs_attention', deliveryHealth: 'healthy' }),
    ).toBe('needs_attention');
    expect(suiteConnectionState({ ...link, deliveryHealth: 'needs_attention' })).toBe(
      'needs_attention',
    );
    expect(suiteConnectionState({ ...link, deliveryHealth: 'disabled' })).toBe('disabled');
    expect(suiteConnectionState({ ...link, status: 'revoked' })).toBe('not_connected');
  });
  it('preserves API transport error codes and maps uncoded errors to unavailable', async () => {
    const f = sessionFixture();
    const connected = new ConnectedApps(f.client, f.session);
    f.request.mockRejectedValueOnce(
      Object.assign(new Error('Account mismatch'), { code: 'suite_link_account_mismatch' }),
    );
    expect(await connected.load()).toEqual({
      type: 'failed',
      code: 'suite_link_account_mismatch',
    });
    f.request.mockRejectedValueOnce(new Error('Network failure'));
    expect(await connected.load()).toEqual({ type: 'failed', code: 'suite_unavailable' });
  });
});
it('keeps all locales and interpolation parameters in parity', () => {
  for (const locale of ['de', 'es'] as const) {
    expect(compareCatalogKeys(suiteMessages.en, suiteMessages[locale])).toEqual({
      missing: [],
      extra: [],
    });
    for (const key of Object.keys(suiteMessages.en) as (keyof typeof suiteMessages.en)[]) {
      const placeholders = (text: string) =>
        [...text.matchAll(/\{([^}]+)\}/g)].map((match) => match[1]).sort();
      expect(placeholders(suiteMessages[locale][key])).toEqual(placeholders(suiteMessages.en[key]));
    }
  }
});
