import { describe, expect, it, vi } from 'vitest';

import { AccountPlanError, createAccountPlanBackend } from './index.js';

const entitlement = {
  mode: 'hub',
  suiteSubject: 'suite|a',
  tier: 'premium',
  features: ['hebkit.export'],
  revision: 3,
  plansUrl: 'https://accounts.example.test/plans',
  billing: { channel: 'app_store', soldBy: 'hebkit' },
};
const balance = {
  suiteSubject: 'suite|a',
  wallet: 'hebkit',
  revision: 0,
  total: 10,
  grant: { available: 0, expiresAt: null },
  packs: [{ available: 10, expiresAt: 1795392000 }],
};

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}
function error(status: number, code: string, headers: Record<string, string> = {}): Response {
  return json(status, { error: { code, message: code, details: {} } }, headers);
}

describe('createAccountPlanBackend', () => {
  it('reads both routes under the API prefix with per-request headers', async () => {
    const fetch = vi.fn((url: string) =>
      Promise.resolve(json(200, url.endsWith('/me/credits') ? balance : entitlement)),
    );
    const backend = createAccountPlanBackend({
      baseUrl: 'https://api.example.test/api/v1/',
      fetch,
      headers: () => ({ authorization: 'Bearer t' }),
    });

    await expect(backend.refreshEntitlement()).resolves.toEqual(entitlement);
    await expect(backend.creditBalance?.()).resolves.toEqual(balance);
    expect(fetch).toHaveBeenCalledWith('https://api.example.test/api/v1/me/entitlement', {
      headers: { accept: 'application/json', authorization: 'Bearer t' },
    });
    expect(fetch.mock.calls[1]?.[0]).toBe('https://api.example.test/api/v1/me/credits');
  });

  it('treats a session without suite identity as no entitlement', async () => {
    const backend = createAccountPlanBackend({
      baseUrl: 'https://api.example.test',
      fetch: () => Promise.resolve(error(403, 'suite_identity_missing')),
    });
    await expect(backend.refreshEntitlement()).resolves.toBeNull();
  });

  it('raises the plan error code with Retry-After', async () => {
    const backend = createAccountPlanBackend({
      baseUrl: 'https://api.example.test',
      fetch: () => Promise.resolve(error(503, 'entitlements_unavailable', { 'retry-after': '5' })),
    });
    await expect(backend.refreshEntitlement()).rejects.toEqual(
      new AccountPlanError(503, 'entitlements_unavailable', 5),
    );
    const broken = createAccountPlanBackend({
      baseUrl: 'https://api.example.test',
      fetch: () => Promise.resolve(new Response('oops', { status: 502 })),
    });
    await expect(broken.creditBalance?.()).rejects.toMatchObject({
      status: 502,
      code: 'internal_error',
      retryAfterSeconds: null,
    });
  });

  it('rejects bodies that do not match the route shapes', async () => {
    for (const [path, body] of [
      ['entitlement', { ...entitlement, mode: 'local' }],
      ['entitlement', { ...entitlement, revision: -1 }],
      ['entitlement', { ...entitlement, billing: 'web' }],
      ['credits', { ...balance, available: 10, total: undefined }],
      ['credits', { ...balance, packs: [{ remaining: 1 }] }],
    ] as const) {
      const backend = createAccountPlanBackend({
        baseUrl: 'https://api.example.test',
        fetch: () => Promise.resolve(json(200, body)),
      });
      const read =
        path === 'entitlement' ? backend.refreshEntitlement() : backend.creditBalance?.();
      await expect(read).rejects.toThrow(TypeError);
    }
  });
});
