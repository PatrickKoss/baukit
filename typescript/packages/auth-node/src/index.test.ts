/// <reference types="node" />

import { chmod, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  AuthNodeError,
  decodeDisplayOnlyClaims,
  DeviceFlowClient,
  NodeTokenCache,
  safeAuthNodeErrorMessage,
  type CachedTokenProfile,
} from './index.js';

const ISSUER = 'https://identity.example.test/realms/example';
const DEVICE_ENDPOINT = `${ISSUER}/protocol/openid-connect/auth/device`;
const TOKEN_ENDPOINT = `${ISSUER}/protocol/openid-connect/token`;
const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

async function cachePath(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-'));
  temporaryDirectories.push(directory);
  return join(directory, 'tokens.json');
}

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

function discovery(overrides: Record<string, unknown> = {}): Response {
  return json({
    issuer: ISSUER,
    device_authorization_endpoint: DEVICE_ENDPOINT,
    token_endpoint: TOKEN_ENDPOINT,
    ...overrides,
  });
}

function device(overrides: Record<string, unknown> = {}): Response {
  return json({
    device_code: 'device-secret',
    user_code: 'ABCD-EFGH',
    verification_uri: 'https://identity.example.test/realms/example/device',
    verification_uri_complete:
      'https://identity.example.test/realms/example/device?user_code=ABCD-EFGH',
    expires_in: 600,
    interval: 1,
    ...overrides,
  });
}

function client(
  path: string,
  fetchImplementation: typeof fetch,
  options: {
    readonly now?: () => number;
    readonly sleep?: (milliseconds: number, signal?: AbortSignal) => Promise<void>;
    readonly requestTimeoutMs?: number;
    readonly loginTimeoutMs?: number;
    readonly issuer?: string;
    readonly endpointPolicy?: { readonly allowLoopbackHttp?: boolean };
  } = {},
): DeviceFlowClient {
  return new DeviceFlowClient(
    {
      issuer: options.issuer ?? ISSUER,
      clientId: 'example-cli',
      scopes: ['profile', 'custom', 'profile'],
      audience: 'example-api',
      cache: { namespace: 'example-cli', path },
      refreshLeewaySeconds: 0,
      ...(options.requestTimeoutMs === undefined
        ? {}
        : { requestTimeoutMs: options.requestTimeoutMs }),
      ...(options.loginTimeoutMs === undefined ? {} : { loginTimeoutMs: options.loginTimeoutMs }),
      ...(options.endpointPolicy === undefined ? {} : { endpointPolicy: options.endpointPolicy }),
    },
    {
      fetch: fetchImplementation,
      now: options.now,
      sleep: options.sleep,
    },
  );
}

function form(init: RequestInit | undefined): URLSearchParams {
  if (!(init?.body instanceof URLSearchParams)) throw new Error('Expected a form request.');
  return init.body;
}

function profile(overrides: Partial<CachedTokenProfile> = {}): CachedTokenProfile {
  return {
    accessToken: 'old-access-secret',
    refreshToken: 'old-refresh-secret',
    tokenType: 'Bearer',
    expiresAt: 1,
    issuer: ISSUER,
    clientId: 'example-cli',
    scopes: ['openid', 'profile', 'custom'],
    audience: 'example-api',
    ...overrides,
  };
}

describe('device authorization', () => {
  it('uses S256 PKCE, handles pending and slow-down, and accepts an optional refresh token', async () => {
    const path = await cachePath();
    const requests: { readonly url: string; readonly init: RequestInit | undefined }[] = [];
    const sleeps: number[] = [];
    const statuses: string[] = [];
    let now = 1_000;
    let tokenPoll = 0;
    const authFetch: typeof fetch = (input, init) => {
      const url = requestUrl(input);
      requests.push({ url, init });
      if (url.includes('.well-known')) return Promise.resolve(discovery());
      if (url === DEVICE_ENDPOINT) return Promise.resolve(device());
      tokenPoll += 1;
      if (tokenPoll === 1) return Promise.resolve(json({ error: 'authorization_pending' }, 400));
      if (tokenPoll === 2) return Promise.resolve(json({ error: 'slow_down' }, 400));
      return Promise.resolve(json({ access_token: 'new-access-secret', expires_in: 120 }));
    };
    const auth = client(path, authFetch, {
      now: () => now,
      sleep: (milliseconds) => {
        sleeps.push(milliseconds);
        now += milliseconds;
        return Promise.resolve();
      },
    });
    const showVerification = vi.fn();
    const openBrowser = vi.fn();

    const cached = await auth.login({
      presentation: {
        showVerification,
        openBrowser,
        showStatus: (status) => {
          statuses.push(status);
        },
      },
    });

    expect(cached).toMatchObject({ accessToken: 'new-access-secret', expiresAt: 129_000 });
    expect(cached.refreshToken).toBeUndefined();
    expect(sleeps).toEqual([1_000, 1_000, 6_000]);
    expect(statuses).toEqual(['waiting', 'slow-down', 'authorized']);
    expect(showVerification).toHaveBeenCalledWith({
      verificationUri: 'https://identity.example.test/realms/example/device',
      verificationUriComplete:
        'https://identity.example.test/realms/example/device?user_code=ABCD-EFGH',
      userCode: 'ABCD-EFGH',
    });
    expect(openBrowser).toHaveBeenCalledWith(
      'https://identity.example.test/realms/example/device?user_code=ABCD-EFGH',
    );
    const deviceRequest = form(requests[1]?.init);
    const tokenRequest = form(requests.at(-1)?.init);
    expect(deviceRequest.get('scope')).toBe('openid profile custom');
    expect(deviceRequest.get('audience')).toBe('example-api');
    expect(deviceRequest.get('code_challenge_method')).toBe('S256');
    expect(deviceRequest.get('code_challenge')).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    expect(tokenRequest.get('code_verifier')).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    expect(tokenRequest.get('code_verifier')).not.toBe(deviceRequest.get('code_challenge'));
  });

  it.each([
    ['access_denied', 'device_authorization_denied'],
    ['expired_token', 'device_authorization_expired'],
    ['invalid_grant', 'device_authorization_failed'],
  ] as const)('maps %s to %s without provider details', async (providerCode, expectedCode) => {
    const authFetch: typeof fetch = (input) => {
      const url = requestUrl(input);
      if (url.includes('.well-known')) return Promise.resolve(discovery());
      if (url === DEVICE_ENDPOINT) return Promise.resolve(device());
      return Promise.resolve(
        json({ error: providerCode, error_description: 'token=private' }, 400),
      );
    };
    const auth = client(await cachePath(), authFetch, { sleep: () => Promise.resolve() });
    const error = await auth.login().catch((cause: unknown) => cause);
    expect(error).toMatchObject({ code: expectedCode });
    expect(String(error)).not.toContain('private');
  });

  it('expires locally when the device lifetime elapses', async () => {
    let now = 0;
    const authFetch: typeof fetch = (input) => {
      const url = requestUrl(input);
      if (url.includes('.well-known')) return Promise.resolve(discovery());
      return Promise.resolve(device({ expires_in: 1 }));
    };
    const auth = client(await cachePath(), authFetch, {
      now: () => now,
      sleep: (milliseconds) => {
        now += milliseconds;
        return Promise.resolve();
      },
    });
    await expect(auth.login()).rejects.toMatchObject({ code: 'device_authorization_expired' });
  });

  it.each([
    [{ expires_in: '600' }, 'invalid_device_response'],
    [{ expires_in: Number.NaN }, 'invalid_device_response'],
    [{ interval: 0 }, 'invalid_device_response'],
    [{ interval: 1.5 }, 'invalid_device_response'],
  ])('rejects malformed device numeric fields %#', async (overrides, code) => {
    const authFetch: typeof fetch = (input) =>
      Promise.resolve(requestUrl(input).includes('.well-known') ? discovery() : device(overrides));
    await expect(client(await cachePath(), authFetch).login()).rejects.toMatchObject({ code });
  });

  it('rejects a malformed token lifetime', async () => {
    const authFetch: typeof fetch = (input) => {
      const url = requestUrl(input);
      if (url.includes('.well-known')) return Promise.resolve(discovery());
      if (url === DEVICE_ENDPOINT) return Promise.resolve(device());
      return Promise.resolve(json({ access_token: 'private-token', expires_in: '300' }));
    };
    await expect(
      client(await cachePath(), authFetch, { sleep: () => Promise.resolve() }).login(),
    ).rejects.toMatchObject({ code: 'invalid_token_response' });
  });
});

describe('issuer and endpoint policy', () => {
  it('rejects an issuer mismatch unless the discovered alias is explicitly allowed', async () => {
    const alias = 'https://identity.example.test/realms/alias';
    const authFetch: typeof fetch = () =>
      Promise.resolve(
        discovery({
          issuer: alias,
          token_endpoint: `${alias}/token`,
          device_authorization_endpoint: `${alias}/device`,
        }),
      );
    const path = await cachePath();
    await expect(client(path, authFetch).discover()).rejects.toMatchObject({
      code: 'issuer_mismatch',
    });
    const allowed = new DeviceFlowClient(
      {
        issuer: ISSUER,
        clientId: 'client',
        cache: { namespace: 'test', path },
        endpointPolicy: { issuerAllowlist: [alias] },
      },
      { fetch: authFetch },
    );
    await expect(allowed.discover()).resolves.toMatchObject({ issuer: alias });
  });

  it.each([
    ['http://identity.example.test/realms/example/token'],
    ['https://other.example.test/token'],
    ['https://identity.example.test/unrelated/token'],
  ])('rejects unsafe or unrelated endpoint %s', async (tokenEndpoint) => {
    const authFetch: typeof fetch = () =>
      Promise.resolve(discovery({ token_endpoint: tokenEndpoint }));
    await expect(client(await cachePath(), authFetch).discover()).rejects.toMatchObject({
      code: 'endpoint_policy_violation',
    });
  });

  it('allows HTTP only for an explicit loopback development policy', async () => {
    const loopbackIssuer = 'http://127.0.0.1:9999/realms/test';
    const authFetch: typeof fetch = () =>
      Promise.resolve(
        json({
          issuer: loopbackIssuer,
          token_endpoint: `${loopbackIssuer}/token`,
          device_authorization_endpoint: `${loopbackIssuer}/device`,
        }),
      );
    expect(() => client('/tmp/not-read', authFetch, { issuer: loopbackIssuer })).toThrow(
      AuthNodeError,
    );
    await expect(
      client(await cachePath(), authFetch, {
        issuer: loopbackIssuer,
        endpointPolicy: { allowLoopbackHttp: true },
      }).discover(),
    ).resolves.toMatchObject({ issuer: loopbackIssuer });
  });
});

describe('response and cancellation bounds', () => {
  it('rejects declared and streamed oversized responses without retaining their contents', async () => {
    const declared: typeof fetch = () =>
      Promise.resolve(
        new Response('{}', { headers: { 'content-length': '70000', 'x-private': 'secret' } }),
      );
    await expect(client(await cachePath(), declared).discover()).rejects.toMatchObject({
      code: 'invalid_discovery_document',
    });

    const streamed: typeof fetch = () => Promise.resolve(json({ padding: 'x'.repeat(70_000) }));
    const error = await client(await cachePath(), streamed)
      .discover()
      .catch((cause: unknown) => cause);
    expect(error).toMatchObject({ code: 'invalid_discovery_document' });
    expect(String(error)).not.toContain('xxxxx');
  });

  it('supports caller abort during polling', async () => {
    const controller = new AbortController();
    const authFetch: typeof fetch = (input) =>
      Promise.resolve(requestUrl(input).includes('.well-known') ? discovery() : device());
    const auth = client(await cachePath(), authFetch, {
      sleep: (_milliseconds, signal) => {
        controller.abort();
        return signal?.aborted === true
          ? Promise.reject(new AuthNodeError('aborted'))
          : Promise.resolve();
      },
    });
    await expect(auth.login({ signal: controller.signal })).rejects.toMatchObject({
      code: 'aborted',
    });
  });

  it('enforces the per-request timeout', async () => {
    const never: typeof fetch = (_input, init) =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    await expect(
      client(await cachePath(), never, { requestTimeoutMs: 5, loginTimeoutMs: 1_000 }).login(),
    ).rejects.toMatchObject({ code: 'login_timeout' });
  });

  it('enforces the total login timeout while polling', async () => {
    const authFetch: typeof fetch = (input) =>
      Promise.resolve(requestUrl(input).includes('.well-known') ? discovery() : device());
    await expect(
      client(await cachePath(), authFetch, {
        loginTimeoutMs: 5,
        sleep: (_milliseconds, signal) =>
          new Promise((_resolve, reject) => {
            signal?.addEventListener('abort', () => {
              reject(new DOMException('aborted', 'AbortError'));
            });
          }),
      }).login(),
    ).rejects.toMatchObject({ code: 'login_timeout' });
  });
});

describe('refresh lifecycle', () => {
  it('deduplicates concurrent refreshes across clients and preserves an omitted refresh token', async () => {
    const path = await cachePath();
    const cache = new NodeTokenCache({ namespace: 'example-cli', path });
    await cache.write('default', profile());
    let refreshCalls = 0;
    const authFetch: typeof fetch = async (input) => {
      const url = requestUrl(input);
      if (url.includes('.well-known')) return discovery();
      refreshCalls += 1;
      await Promise.resolve();
      return json({ access_token: 'refreshed-access-secret', expires_in: 300 });
    };
    const first = client(path, authFetch, { now: () => 10_000 });
    const second = client(path, authFetch, { now: () => 10_000 });

    await expect(
      Promise.all([first.accessToken(), first.accessToken(), second.accessToken()]),
    ).resolves.toEqual([
      'refreshed-access-secret',
      'refreshed-access-secret',
      'refreshed-access-secret',
    ]);
    expect(refreshCalls).toBe(1);
    expect(await cache.read()).toMatchObject({
      accessToken: 'refreshed-access-secret',
      refreshToken: 'old-refresh-secret',
    });
  });

  it('stores a rotated refresh token', async () => {
    const path = await cachePath();
    const cache = new NodeTokenCache({ namespace: 'example-cli', path });
    await cache.write('default', profile());
    const authFetch: typeof fetch = (input) =>
      Promise.resolve(
        requestUrl(input).includes('.well-known')
          ? discovery()
          : json({
              access_token: 'rotated-access-secret',
              refresh_token: 'rotated-refresh-secret',
              expires_in: 300,
            }),
      );
    await expect(client(path, authFetch, { now: () => 10_000 }).accessToken()).resolves.toBe(
      'rotated-access-secret',
    );
    expect(await cache.read()).toMatchObject({ refreshToken: 'rotated-refresh-secret' });
  });

  it('uses an injected environment token before reading the cache', async () => {
    const auth = new DeviceFlowClient(
      {
        issuer: ISSUER,
        clientId: 'client',
        cache: { namespace: 'test', path: '/does/not/exist/tokens.json' },
      },
      { environmentToken: () => ' environment-secret ' },
    );
    await expect(auth.accessToken()).resolves.toBe('environment-secret');
  });

  it('does not use a cache profile bound to different client inputs', async () => {
    const path = await cachePath();
    await new NodeTokenCache({ namespace: 'example-cli', path }).write(
      'default',
      profile({
        expiresAt: 100_000,
        scopes: ['openid', 'profile', 'custom'],
        audience: 'different-api',
      }),
    );
    await expect(client(path, vi.fn(), { now: () => 10_000 }).accessToken()).rejects.toMatchObject({
      code: 'no_cached_session',
    });
  });

  it('maps a refresh transport failure to a stable error', async () => {
    const path = await cachePath();
    await new NodeTokenCache({ namespace: 'example-cli', path }).write('default', profile());
    const authFetch: typeof fetch = (input) => {
      if (requestUrl(input).includes('.well-known')) return Promise.resolve(discovery());
      return Promise.reject(new TypeError('request included private provider text'));
    };
    const error = await client(path, authFetch, { now: () => 10_000 })
      .accessToken()
      .catch((cause: unknown) => cause);
    expect(error).toMatchObject({ code: 'refresh_failed', retryable: true });
    expect(String(error)).not.toContain('private provider text');
  });
});

describe('cache safety', () => {
  it('stores separate profiles and logs out only the selected profile', async () => {
    const path = await cachePath();
    const cache = new NodeTokenCache({ namespace: 'example-cli', path });
    await cache.write('work', profile({ accessToken: 'work-secret' }));
    await cache.write('personal', profile({ accessToken: 'personal-secret' }));
    await expect(cache.remove('work')).resolves.toBe(true);
    await expect(cache.remove('work')).resolves.toBe(false);
    expect(await cache.read('work')).toBeUndefined();
    expect(await cache.read('personal')).toMatchObject({ accessToken: 'personal-secret' });
  });

  it('reports corrupt cache data with a stable safe error', async () => {
    const path = await cachePath();
    await writeFile(path, '{"accessToken":"private-secret"', { mode: 0o600 });
    const error = await new NodeTokenCache({ namespace: 'test', path })
      .read()
      .catch((cause: unknown) => cause);
    expect(error).toMatchObject({ code: 'cache_corrupt' });
    expect(safeAuthNodeErrorMessage(error)).not.toContain('private-secret');
  });

  it('rejects group-readable cache files on POSIX and ignores mode bits on Windows', async () => {
    const path = await cachePath();
    const cache = new NodeTokenCache({ namespace: 'test', path });
    await cache.write('default', profile());
    await chmod(path, 0o644);
    await expect(cache.read()).rejects.toMatchObject({ code: 'cache_permission' });
    await expect(
      new NodeTokenCache({ namespace: 'test', path, platform: 'win32' }).read(),
    ).resolves.toMatchObject({
      accessToken: 'old-access-secret',
    });
  });

  it('rejects a symlink cache and a symlink parent', async () => {
    if (process.platform === 'win32') return;
    const directory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-links-'));
    temporaryDirectories.push(directory);
    const target = join(directory, 'target.json');
    await writeFile(target, '{}', { mode: 0o600 });
    const linkedFile = join(directory, 'linked.json');
    await symlink(target, linkedFile);
    await expect(
      new NodeTokenCache({ namespace: 'test', path: linkedFile }).read(),
    ).rejects.toMatchObject({
      code: 'cache_symlink',
    });
    const realDirectory = join(directory, 'real');
    await mkdir(realDirectory, { mode: 0o700 });
    const linkedDirectory = join(directory, 'linked-directory');
    await symlink(realDirectory, linkedDirectory);
    await expect(
      new NodeTokenCache({ namespace: 'test', path: join(linkedDirectory, 'tokens.json') }).write(
        'default',
        profile(),
      ),
    ).rejects.toMatchObject({ code: 'cache_symlink' });
  });

  it('preserves the old cache when a replacement is interrupted by a permission failure', async () => {
    if (process.platform === 'win32') return;
    const path = await cachePath();
    const cache = new NodeTokenCache({ namespace: 'test', path });
    await cache.write('default', profile());
    const directory = join(path, '..');
    await chmod(directory, 0o500);
    const failure = await cache
      .write('default', profile({ accessToken: 'replacement-secret' }))
      .catch((cause: unknown) => cause);
    await chmod(directory, 0o700);
    expect(failure).toMatchObject({ code: 'cache_permission' });
    expect(await cache.read()).toMatchObject({ accessToken: 'old-access-secret' });
    expect(await readFile(path, 'utf8')).not.toContain('replacement-secret');
  });

  it('times out on an existing cross-process lock', async () => {
    const path = await cachePath();
    await writeFile(`${path}.lock`, 'locked', { mode: 0o600 });
    let now = 0;
    const cache = new NodeTokenCache({
      namespace: 'test',
      path,
      now: () => now,
      sleep: (milliseconds) => {
        now += milliseconds;
        return Promise.resolve();
      },
      lockTimeoutMs: 2,
      lockRetryMs: 1,
    });
    await expect(cache.write('default', profile())).rejects.toMatchObject({
      code: 'cache_lock_timeout',
    });
  });
});

describe('display-only claims', () => {
  it('decodes only named unverified display hints', () => {
    const encoded = Buffer.from(
      JSON.stringify({
        sub: 'user-1',
        name: 'Test User',
        preferred_username: 'test',
        email: 'test@example.test',
        exp: 123,
        roles: ['admin'],
      }),
    ).toString('base64url');
    expect(decodeDisplayOnlyClaims(`header.${encoded}.signature`)).toEqual({
      subject: 'user-1',
      name: 'Test User',
      preferredUsername: 'test',
      email: 'test@example.test',
      expiresAt: 123,
    });
    expect(decodeDisplayOnlyClaims('not-a-jwt')).toBeUndefined();
  });
});

function requestUrl(input: string | URL | Request): string {
  return input instanceof Request ? input.url : input.toString();
}
