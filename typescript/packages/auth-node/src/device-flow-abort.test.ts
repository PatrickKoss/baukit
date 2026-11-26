import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { DeviceFlowClient, NodeTokenCache } from './index.js';

const ISSUER = 'https://identity.example.test/realms/example';
const TOKEN_ENDPOINT = `${ISSUER}/protocol/openid-connect/token`;
const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true })));
});

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), { headers: { 'content-type': 'application/json' } });
}

describe('DeviceFlowClient refresh abort', () => {
  it('stores the rotated refresh token when the caller aborts mid-request', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-abort-'));
    directories.push(directory);
    const path = join(directory, 'tokens.json');
    const cache = new NodeTokenCache({ namespace: 'example-cli', path });
    await cache.write('default', {
      accessToken: 'expired-access',
      refreshToken: 'refresh-1',
      tokenType: 'Bearer',
      issuedAt: 0,
      expiresAt: 1,
      issuer: ISSUER,
      clientId: 'example-cli',
      scopes: ['openid', 'profile', 'offline_access'],
    });
    const controller = new AbortController();
    const fetch: typeof globalThis.fetch = (input, init) => {
      const url = input instanceof Request ? input.url : String(input);
      if (url.includes('.well-known')) {
        return Promise.resolve(
          json({
            issuer: ISSUER,
            token_endpoint: TOKEN_ENDPOINT,
            device_authorization_endpoint: `${ISSUER}/device`,
          }),
        );
      }
      controller.abort();
      if (init?.signal?.aborted === true) {
        return Promise.reject(new DOMException('aborted', 'AbortError'));
      }
      return Promise.resolve(
        json({
          access_token: 'access-2',
          refresh_token: 'refresh-2',
          token_type: 'Bearer',
          expires_in: 600,
        }),
      );
    };
    const client = new DeviceFlowClient(
      { issuer: ISSUER, clientId: 'example-cli', cache: { namespace: 'example-cli', path } },
      { fetch, now: () => 10_000, sleep: () => Promise.resolve() },
    );

    await client.accessToken({ signal: controller.signal }).catch(() => undefined);

    await expect(cache.read()).resolves.toMatchObject({ refreshToken: 'refresh-2' });
  });
});
