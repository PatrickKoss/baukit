import { exportJWK, generateKeyPair, SignJWT } from 'jose';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { SilentTokenBroker, SilentTokenError } from './index.js';
import type { SilentTokenEnvironment, SilentTokenProduct } from './index.js';

const product: SilentTokenProduct = {
  id: 'one',
  issuer: 'https://id.example/realms/one',
  clientId: 'consent',
  audience: 'one-api',
  redirectUri: 'https://account.example/product-token-callback.html',
};
let keys: Awaited<ReturnType<typeof generateKeyPair>>;
let jwks: { keys: Awaited<ReturnType<typeof exportJWK>>[] };
beforeAll(async () => {
  keys = await generateKeyPair('RS256', { extractable: true });
  jwks = { keys: [{ ...(await exportJWK(keys.publicKey)), kid: 'test', alg: 'RS256' }] };
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function fixture(products = [product]) {
  products = products.map((entry) => ({ ...entry, issuer: entry.issuer.replace(/\/+$/u, '') }));
  let now = Date.now();
  let callback: (url: URL) => URL = (url) => url;
  let idClaims: Record<string, unknown> = {};
  let accessClaims: Record<string, unknown> = {};
  const transactions = new Map<string, URL>();
  const authorizations: URL[] = [];
  const close = vi.fn();
  const receive = vi.fn((url: URL, signal: AbortSignal) => {
    signal.throwIfAborted();
    transactions.set(required(url.searchParams.get('state')), url);
    authorizations.push(url);
    const result = new URL(required(url.searchParams.get('redirect_uri')));
    result.search = new URLSearchParams({
      state: required(url.searchParams.get('state')),
      iss: `${url.origin}${required(url.pathname.split('/protocol')[0])}`,
      code: required(url.searchParams.get('state')),
    }).toString();
    return Promise.resolve(callback(result).toString());
  });
  const open = vi.fn(() => ({ receive, close }));
  const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
    expect(init).toMatchObject({ credentials: 'omit', cache: 'no-store', redirect: 'error' });
    expect(init?.signal).toBeInstanceOf(AbortSignal);
    const url = input instanceof Request ? input.url : input.toString();
    const current = required(products.find((entry) => url.startsWith(entry.issuer)));
    const protocol = `${current.issuer}/protocol/openid-connect`;
    if (url.endsWith('openid-configuration'))
      return Response.json({
        issuer: current.issuer,
        authorization_endpoint: `${protocol}/auth`,
        token_endpoint: `${protocol}/token`,
        jwks_uri: `${protocol}/certs`,
      });
    if (url.endsWith('/certs')) return Response.json(jwks);
    const body = required(init).body as URLSearchParams;
    const authorization = required(transactions.get(required(body.get('code'))));
    const challenge = btoa(
      String.fromCharCode(
        ...new Uint8Array(
          await globalThis.crypto.subtle.digest(
            'SHA-256',
            new TextEncoder().encode(required(body.get('code_verifier'))),
          ),
        ),
      ),
    )
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/u, '');
    expect(challenge).toBe(authorization.searchParams.get('code_challenge'));
    expect(authorization.searchParams.get('code_challenge_method')).toBe('S256');
    expect(body.get('grant_type')).toBe('authorization_code');
    expect(body.get('client_secret')).toBeNull();
    const issued = Math.floor(now / 1000);
    const sign = (claims: Record<string, unknown>, audience: string) =>
      new SignJWT({
        nonce: authorization.searchParams.get('nonce'),
        azp: current.clientId,
        iss: current.issuer,
        aud: audience,
        sub: 'product-user',
        iat: issued,
        exp: issued + 60,
        ...claims,
      })
        .setProtectedHeader({ alg: 'RS256', kid: 'test' })
        .sign(keys.privateKey);
    return Response.json({
      token_type: 'Bearer',
      expires_in: 60,
      access_token: await sign(
        {
          account_id: 'session-user',
          ...accessClaims,
        },
        current.audience,
      ),
      id_token: await sign(idClaims, current.clientId),
      refresh_token: 'must-never-be-retained',
    });
  });
  const environment: SilentTokenEnvironment = {
    fetch,
    crypto: globalThis.crypto,
    now: () => now,
    open,
  };
  const broker = new SilentTokenBroker(
    products,
    { accountBinding: { claim: 'account_id', value: 'session-user' } },
    environment,
  );
  return {
    broker,
    environment,
    open,
    receive,
    close,
    fetch,
    authorizations,
    advance: (ms: number) => {
      now += ms;
    },
    callback: (fn: typeof callback) => {
      callback = fn;
    },
    idClaims: (claims: typeof idClaims) => {
      idClaims = claims;
    },
    accessClaims: (claims: typeof accessClaims) => {
      accessClaims = claims;
    },
  };
}

it('caches in memory, shares parallel requests and renews before expiry', async () => {
  const f = fixture();
  const first = f.broker.getToken('one');
  expect(f.broker.getToken('one')).toBe(first);
  const token = await first;
  expect(token.status).toBe('ok');
  expect(await f.broker.getToken('one')).toBe(token);
  expect(f.open).toHaveBeenCalledTimes(1);
  expect(required(f.authorizations[0]).searchParams.get('prompt')).toBe('none');
  f.advance(31_000);
  await f.broker.getToken('one');
  expect(f.open).toHaveBeenCalledTimes(2);
  expect(f.close).toHaveBeenCalledTimes(2);
  f.broker.invalidate('one');
  await f.broker.getToken('one');
  expect(f.open).toHaveBeenCalledTimes(3);
  f.broker.dispose();
  expect(() => f.broker.getToken('one')).toThrow('disposed');
});

it('partitions tokens by product issuer and configured API audience', async () => {
  const second = {
    ...product,
    id: 'two',
    issuer: 'https://other.example/realms/two',
    audience: 'two-api',
  };
  const f = fixture([product, second]);
  const [one, two] = await Promise.all([
    f.broker.getToken('one', 'one-api'),
    f.broker.getToken('two', 'two-api'),
  ]);
  expect(one).not.toEqual(two);
  expect(f.open).toHaveBeenCalledTimes(2);
  expect(() => f.broker.getToken('one', 'two-api')).toThrow('unknown_product_or_audience');
  expect(() => f.broker.getToken('missing')).toThrow('unknown_product_or_audience');
});

it.each(['login_required', 'interaction_required', 'consent_required'] as const)(
  'latches %s until explicit login succeeds',
  async (reason) => {
    const f = fixture();
    f.callback((url) => {
      url.searchParams.set('error', reason);
      url.searchParams.delete('code');
      return url;
    });
    const failure = await f.broker.getToken('one');
    expect(failure).toMatchObject(
      reason === 'consent_required'
        ? { status: reason, productId: 'one' }
        : { status: 'login_required', productId: 'one', reason },
    );
    f.broker.invalidate('one');
    expect(await f.broker.getToken('one')).toBe(failure);
    expect(f.open).toHaveBeenCalledTimes(1);
    f.callback((url) => url);
    const login = f.broker.login('one');
    expect(f.open).toHaveBeenLastCalledWith(true);
    expect(f.broker.login('one')).toBe(login);
    expect((await login).status).toBe('ok');
    expect(required(f.authorizations[1]).searchParams.has('prompt')).toBe(false);
    expect((await f.broker.getToken('one')).status).toBe('ok');
  },
);

it('times out, closes the frame and does not try again automatically', async () => {
  vi.useFakeTimers();
  const f = fixture();
  f.receive.mockImplementation(
    (_url, signal) =>
      new Promise((_resolve, reject) => {
        signal.addEventListener(
          'abort',
          () => {
            reject(signal.reason instanceof Error ? signal.reason : new Error('aborted'));
          },
          { once: true },
        );
      }),
  );
  const pending = f.broker.getToken('one');
  await vi.waitFor(() => {
    expect(f.receive).toHaveBeenCalled();
  });
  await vi.advanceTimersByTimeAsync(15_000);
  const result = await pending;
  expect(result).toMatchObject({ status: 'login_required', reason: 'timeout' });
  expect(await f.broker.getToken('one')).toBe(result);
  expect(f.close).toHaveBeenCalledTimes(1);
});

it.each([
  [
    'invalid_state',
    (url: URL) => {
      url.searchParams.set('state', 'forged');
    },
  ],
  [
    'invalid_issuer',
    (url: URL) => {
      url.searchParams.set('iss', 'https://wrong.example');
    },
  ],
  [
    'invalid_callback',
    (url: URL) => {
      url.searchParams.append('state', 'duplicate');
    },
  ],
  [
    'invalid_callback',
    (url: URL) => {
      url.pathname = '/wrong';
    },
  ],
  [
    'missing_code',
    (url: URL) => {
      url.searchParams.delete('code');
    },
  ],
  [
    'authorization_rejected',
    (url: URL) => {
      url.searchParams.set('error', '<untrusted>');
      url.searchParams.delete('code');
    },
  ],
] as const)('rejects callback %s before exchanging a code', async (code, mutate) => {
  const f = fixture();
  f.callback((url) => {
    mutate(url);
    return url;
  });
  expect(await f.broker.getToken('one')).toMatchObject({ status: 'error', error: { code } });
  expect(f.fetch).toHaveBeenCalledTimes(1);
  expect(f.close).toHaveBeenCalledTimes(1);
});

it('rejects a signed ID token with the wrong nonce', async () => {
  const f = fixture();
  f.idClaims({ nonce: 'wrong' });
  expect(await f.broker.getToken('one')).toMatchObject({
    status: 'error',
    error: { code: 'invalid_nonce' },
  });
});
it.each([
  { account_id: 'another-session-user' },
  { account_id: undefined },
  { azp: undefined },
  { azp: 'other-client' },
  { sub: 'other-product-user' },
])('rejects account and client mismatches %j', async (claims) => {
  const f = fixture();
  f.accessClaims(claims);
  expect(await f.broker.getToken('one')).toMatchObject({ status: 'account_mismatch' });
});
it.each([{ exp: 1 }, { aud: 'other-audience' }, { iss: 'https://wrong.example' }])(
  'rejects invalid signed claims %j',
  async (claims) => {
    const f = fixture();
    f.accessClaims(claims);
    const original = required(f.fetch.getMockImplementation());
    f.fetch.mockImplementation(async (...args) => {
      const response = await original(...args);
      if (!(args[0] instanceof Request ? args[0].url : args[0].toString()).endsWith('/token'))
        return response;
      const body = (await response.json()) as Record<string, unknown>;
      body['access_token'] = await new SignJWT({
        sub: 'product-user',
        azp: 'consent',
        account_id: 'session-user',
        exp: Math.floor(Date.now() / 1000) + 60,
        iat: Math.floor(Date.now() / 1000),
        aud: product.audience,
        iss: product.issuer,
        ...claims,
      })
        .setProtectedHeader({ alg: 'RS256', kid: 'test' })
        .sign(keys.privateKey);
      return Response.json(body);
    });
    const result = await f.broker.getToken('one');
    expect(result.status).toBe('error');
    if (result.status !== 'error') throw new Error('Expected error');
    expect(result.error).toBeInstanceOf(SilentTokenError);
  },
);

it('never calls browser storage APIs and a new instance has no cached tokens', async () => {
  for (const name of ['localStorage', 'sessionStorage', 'indexedDB']) {
    vi.stubGlobal(
      name,
      new Proxy(
        {},
        {
          get: () => {
            throw new Error(`Unexpected ${name} access`);
          },
        },
      ),
    );
  }
  vi.stubGlobal('document', {
    get cookie() {
      throw new Error('Unexpected cookie access');
    },
    set cookie(_value: string) {
      throw new Error('Unexpected cookie write');
    },
  });
  const f = fixture();
  await f.broker.getToken('one');
  const newBroker = new SilentTokenBroker(
    [product],
    { accountBinding: { claim: 'account_id', value: 'session-user' } },
    f.environment,
  );
  await newBroker.getToken('one');
  expect(f.open).toHaveBeenCalledTimes(2);
});

it('aborts work on disposal and cannot accept a late token', async () => {
  const f = fixture();
  f.receive.mockImplementation(
    (_url, signal) =>
      new Promise((_resolve, reject) => {
        signal.addEventListener(
          'abort',
          () => {
            reject(signal.reason instanceof Error ? signal.reason : new Error('aborted'));
          },
          { once: true },
        );
      }),
  );
  const pending = f.broker.getToken('one');
  await vi.waitFor(() => {
    expect(f.receive).toHaveBeenCalled();
  });
  f.broker.dispose();
  expect(await pending).toMatchObject({ status: 'error', error: { code: 'disposed' } });
  expect(f.close).toHaveBeenCalled();
});

describe('configuration and provider failures', () => {
  it('rejects unsafe or duplicate config', () => {
    const f = fixture();
    expect(
      () =>
        new SilentTokenBroker(
          [product, product],
          { accountBinding: { claim: 'account_id', value: 'session-user' } },
          f.environment,
        ),
    ).toThrow('invalid_product');
    expect(
      () =>
        new SilentTokenBroker(
          [product],
          { accountBinding: { claim: 'account_id', value: '' } },
          f.environment,
        ),
    ).toThrow('invalid_account_binding');
    expect(
      () =>
        new SilentTokenBroker(
          [{ ...product, issuer: 'http://unsafe.example' }],
          { accountBinding: { claim: 'account_id', value: 'session-user' } },
          f.environment,
        ),
    ).toThrow('invalid_url');
  });
  it.each([
    Response.json({ issuer: 'https://wrong.example' }),
    new Response('', { status: 500 }),
    Response.json([]),
  ])('rejects bad discovery', async (response) => {
    const f = fixture();
    f.fetch.mockResolvedValue(response);
    const result = await f.broker.getToken('one');
    expect(result.status).toBe('error');
    if (result.status !== 'error') throw new Error('Expected error');
    expect(result.error).toBeInstanceOf(SilentTokenError);
    expect(f.close).toHaveBeenCalled();
  });
});

function required<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error('Missing fixture value');
  return value;
}

it('rejects a token whose signature was forged', async () => {
  const f = fixture();
  const original = required(f.fetch.getMockImplementation());
  f.fetch.mockImplementation(async (...args) => {
    const response = await original(...args);
    const url = args[0] instanceof Request ? args[0].url : args[0].toString();
    if (!url.endsWith('/token')) return response;
    const body = (await response.json()) as Record<string, unknown>;
    const token = String(body['id_token']).split('.');
    token[2] = 'invalid-signature';
    return Response.json({ ...body, id_token: token.join('.') });
  });
  expect(await f.broker.getToken('one')).toMatchObject({
    status: 'error',
    error: { code: 'unavailable_or_invalid_response' },
  });
});

it('resets remembered failures only when requested', async () => {
  const f = fixture();
  f.callback((url) => {
    url.searchParams.set('error', 'login_required');
    url.searchParams.delete('code');
    return url;
  });
  const failure = await f.broker.getToken('one');
  expect(await f.broker.getToken('one')).toBe(failure);
  f.broker.resetFailure('one');
  f.callback((url) => url);
  expect((await f.broker.getToken('one')).status).toBe('ok');
  expect(f.open).toHaveBeenCalledTimes(2);
});

it('remembers sanitized errors and permits a deliberate retry', async () => {
  const f = fixture();
  f.fetch.mockRejectedValueOnce(new Error('private provider content'));
  const result = await f.broker.getToken('one');
  expect(result).toMatchObject({
    status: 'error',
    error: { code: 'unavailable_or_invalid_response' },
  });
  expect(JSON.stringify(result)).not.toContain('private provider content');
  expect(await f.broker.getToken('one')).toBe(result);
  expect(f.open).toHaveBeenCalledTimes(1);
  f.broker.resetFailure('one');
  expect((await f.broker.getToken('one')).status).toBe('ok');
});

it('latches account mismatches and clears them after a successful explicit login', async () => {
  const f = fixture();
  f.accessClaims({ account_id: 'another-user' });
  const result = await f.broker.getToken('one');
  expect(result.status).toBe('account_mismatch');
  expect(await f.broker.getToken('one')).toBe(result);
  f.accessClaims({});
  expect((await f.broker.login('one')).status).toBe('ok');
  expect((await f.broker.getToken('one')).status).toBe('ok');
});

it('keeps products separate even when they share issuer, client and audience', async () => {
  const second = {
    ...product,
    id: 'two',
    scopes: ['profile'],
    redirectUri: 'https://account.example/second-callback',
  };
  const f = fixture([product, second]);
  const first = f.broker.getToken('one');
  const other = f.broker.getToken('two');
  expect(first).not.toBe(other);
  expect((await first).productId).toBe('one');
  expect((await other).productId).toBe('two');
  expect(f.open).toHaveBeenCalledTimes(2);
});

it('requests configured scopes, audience and provider parameters with unique PKCE state and nonce', async () => {
  const f = fixture([
    {
      ...product,
      issuer: `${product.issuer}/`,
      scopes: ['profile', 'openid', 'profile'],
      authorizationParams: { provider_hint: 'organization' },
    },
  ]);
  await f.broker.getToken('one');
  f.broker.invalidate('one');
  await f.broker.getToken('one');
  const first = required(f.authorizations[0]).searchParams;
  const next = required(f.authorizations[1]).searchParams;
  expect(first.get('scope')).toBe('openid profile');
  expect(first.get('audience')).toBe(product.audience);
  expect(first.get('provider_hint')).toBe('organization');
  expect(first.get('response_mode')).toBe('query');
  expect(first.has('kc_idp_hint')).toBe(false);
  for (const field of ['state', 'nonce', 'code_challenge'])
    expect(first.get(field)).not.toBe(next.get(field));
});

it('opens explicit login synchronously and latches popup blocking', async () => {
  const f = fixture();
  f.open.mockImplementationOnce(() => {
    throw new SilentTokenError('popup_blocked');
  });
  const pending = f.broker.login('one');
  expect(f.open).toHaveBeenCalledWith(true);
  expect(f.fetch).not.toHaveBeenCalled();
  const result = await pending;
  expect(result).toMatchObject({ status: 'error', error: { code: 'popup_blocked' } });
  expect(await f.broker.getToken('one')).toBe(result);
  expect((await f.broker.login('one')).status).toBe('ok');
});

it('caps expiry skew at half a short token lifetime and honors the signed expiry', async () => {
  const f = fixture();
  f.accessClaims({ exp: Math.floor(f.environment.now() / 1000) + 10 });
  const token = await f.broker.getToken('one');
  expect(token).toMatchObject({
    status: 'ok',
    expiresAt: Math.floor(f.environment.now() / 1000) * 1000 + 10_000,
  });
  f.advance(4_000);
  expect(await f.broker.getToken('one')).toBe(token);
  f.advance(2_000);
  await f.broker.getToken('one');
  expect(f.open).toHaveBeenCalledTimes(2);
});

it.each(['body', 'JWKS'] as const)(
  'calculates expiry at token response receipt before delayed %s processing',
  async (phase) => {
    const f = fixture();
    f.accessClaims({ exp: Math.floor(f.environment.now() / 1000) + 600 });
    const receivedAt = f.environment.now();
    const original = required(f.fetch.getMockImplementation());
    f.fetch.mockImplementation(async (...args) => {
      const response = await original(...args);
      const url = requestUrl(args[0]);
      if (phase === 'body' && url.endsWith('/token')) {
        const json = response.json.bind(response);
        vi.spyOn(response, 'json').mockImplementation(() => {
          f.advance(20_000);
          return json();
        });
      }
      if (phase === 'JWKS' && url.endsWith('/certs')) f.advance(20_000);
      return response;
    });
    expect(await f.broker.getToken('one')).toMatchObject({
      status: 'ok',
      expiresAt: receivedAt + 60_000,
    });
  },
);

it.each(['discovery', 'callback', 'exchange'] as const)(
  'disposes during %s even if the environment ignores abort',
  async (phase) => {
    const f = fixture();
    let resolveWork: (() => void) | undefined;
    const wait = () =>
      new Promise<void>((resolve) => {
        resolveWork = resolve;
      });
    if (phase === 'callback') {
      const receive = required(f.receive.getMockImplementation());
      f.receive.mockImplementation(async (...args) => {
        await wait();
        return receive(...args);
      });
    } else {
      const fetch = required(f.fetch.getMockImplementation());
      f.fetch.mockImplementation(async (...args) => {
        const isDiscovery = requestUrl(args[0]).endsWith('openid-configuration');
        if (
          (phase === 'discovery' && isDiscovery) ||
          (phase === 'exchange' && requestUrl(args[0]).endsWith('/token'))
        )
          await wait();
        return fetch(...args);
      });
    }
    const pending = f.broker.getToken('one');
    await vi.waitFor(() => {
      expect(resolveWork).toBeTypeOf('function');
    });
    const callsBeforeDispose = f.fetch.mock.calls.length;
    f.broker.dispose();
    f.broker.dispose();
    expect(await pending).toMatchObject({ status: 'error', error: { code: 'disposed' } });
    expect(f.close).toHaveBeenCalledTimes(1);
    required(resolveWork)();
    await new Promise((resolve) => {
      setTimeout(resolve, 25);
    });
    expect(f.fetch).toHaveBeenCalledTimes(callsBeforeDispose);
    for (const operation of [
      () => f.broker.getToken('one'),
      () => f.broker.login('one'),
      () => {
        f.broker.invalidate('one');
      },
      () => {
        f.broker.resetFailure('one');
      },
    ])
      expect(operation).toThrow('disposed');
  },
);

it('bounds explicit login at three minutes', async () => {
  vi.useFakeTimers();
  const f = fixture();
  f.receive.mockImplementation(() => new Promise(() => undefined));
  const pending = f.broker.login('one');
  await vi.waitFor(() => {
    expect(f.receive).toHaveBeenCalled();
  });
  await vi.advanceTimersByTimeAsync(179_000);
  expect(f.close).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1_000);
  expect(await pending).toMatchObject({ status: 'login_required', reason: 'timeout' });
  expect(f.close).toHaveBeenCalledTimes(1);
});

it.each([
  'client_id',
  'redirect_uri',
  'response_type',
  'response_mode',
  'scope',
  'state',
  'nonce',
  'code_challenge',
  'code_challenge_method',
  'prompt',
  'audience',
  'resource',
])('rejects an override of %s', (name) => {
  const f = fixture();
  expect(
    () =>
      new SilentTokenBroker(
        [{ ...product, authorizationParams: { [name]: 'unsafe' } }],
        { accountBinding: { claim: 'account_id', value: 'session-user' } },
        f.environment,
      ),
  ).toThrow('invalid_product');
});

it.each([
  { exp: 1 },
  { aud: 'other-client' },
  { iss: 'https://wrong.example' },
  { azp: 'other-client' },
  { aud: [product.clientId, 'other'], azp: undefined },
])('rejects invalid signed ID token claims %j', async (claims) => {
  const f = fixture();
  f.idClaims(claims);
  expect(await f.broker.getToken('one')).toMatchObject({ status: 'error' });
});

it.each([
  { expires_in: 0 },
  { expires_in: '60' },
  { token_type: 'Basic' },
  { id_token: '' },
  { access_token: '' },
])('rejects malformed token responses %j', async (tokens) => {
  const f = fixture();
  const original = required(f.fetch.getMockImplementation());
  f.fetch.mockImplementation(async (...args) => {
    const response = await original(...args);
    if (!requestUrl(args[0]).endsWith('/token')) return response;
    return Response.json({ ...((await response.json()) as Record<string, unknown>), ...tokens });
  });
  expect(await f.broker.getToken('one')).toMatchObject({
    status: 'error',
    error: { code: 'invalid_tokens' },
  });
});

it.each(['authorization_endpoint', 'token_endpoint', 'jwks_uri'])(
  'rejects a cross-origin discovery %s',
  async (field) => {
    const f = fixture();
    const original = required(f.fetch.getMockImplementation());
    f.fetch.mockImplementation(async (...args) => {
      const response = await original(...args);
      return Response.json({
        ...((await response.json()) as Record<string, unknown>),
        [field]: 'https://evil.example/endpoint',
      });
    });
    expect(await f.broker.getToken('one')).toMatchObject({
      status: 'error',
      error: { code: 'invalid_metadata' },
    });
    expect(f.fetch).toHaveBeenCalledTimes(1);
  },
);

function requestUrl(input: Parameters<typeof globalThis.fetch>[0]): string {
  return input instanceof Request ? input.url : input.toString();
}

it.each([
  [
    'another origin',
    (url: URL) => {
      url.hostname = 'evil.example';
    },
  ],
  [
    'a fragment',
    (url: URL) => {
      url.hash = 'fragment';
    },
  ],
  [
    'URL credentials',
    (url: URL) => {
      url.username = 'user';
    },
  ],
  [
    'duplicate codes',
    (url: URL) => {
      url.searchParams.append('code', 'duplicate');
    },
  ],
  [
    'both code and error',
    (url: URL) => {
      url.searchParams.set('error', 'login_required');
    },
  ],
] as const)('rejects a callback with %s before token exchange', async (_name, mutate) => {
  const f = fixture();
  f.callback((url) => {
    mutate(url);
    return url;
  });
  expect(await f.broker.getToken('one')).toMatchObject({
    status: 'error',
    error: { code: 'invalid_callback' },
  });
  expect(f.fetch).toHaveBeenCalledTimes(1);
});
