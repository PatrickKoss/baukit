/// <reference types="node" />

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { DeviceFlowClient } from './index.js';

describe('RFC 8628 in-process issuer conformance', () => {
  let issuer = '';
  let pollCount = 0;
  let refreshCount = 0;
  let observedChallenge = '';
  let observedVerifier = '';
  let temporaryDirectory = '';
  const server = createServer((request, response) => {
    void route(request, response);
  });

  beforeAll(async () => {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const address = server.address();
    if (address === null || typeof address === 'string') throw new Error('Missing server address.');
    issuer = `http://127.0.0.1:${String(address.port)}/issuer`;
  });

  afterAll(async () => {
    await new Promise<void>((resolve, reject) => {
      server.close((error) => {
        if (error === undefined) resolve();
        else reject(error);
      });
    });
    if (temporaryDirectory.length > 0) {
      await rm(temporaryDirectory, { recursive: true, force: true });
    }
  });

  it('completes discovery, device authorization, pending polling, PKCE, and refresh', async () => {
    temporaryDirectory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-conformance-'));
    const cachePath = join(temporaryDirectory, 'tokens.json');
    const auth = new DeviceFlowClient(
      {
        issuer,
        clientId: 'conformance-client',
        cache: { namespace: 'baukit-auth-node-conformance', path: cachePath },
        endpointPolicy: { allowLoopbackHttp: true },
        refreshLeewaySeconds: 0,
      },
      { sleep: () => Promise.resolve(), now: () => 1_000 },
    );

    const loggedIn = await auth.login();
    expect(loggedIn).toMatchObject({
      accessToken: 'conformance-access',
      refreshToken: 'conformance-refresh',
    });
    expect(pollCount).toBe(2);
    expect(observedChallenge).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    expect(observedVerifier).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    expect(observedVerifier).not.toBe(observedChallenge);
    await expect(auth.accessToken({ forceRefresh: true })).resolves.toBe('conformance-refreshed');
    expect(refreshCount).toBe(1);
    await auth.logout();
  });

  async function route(request: IncomingMessage, response: ServerResponse): Promise<void> {
    if (request.url === '/issuer/.well-known/openid-configuration') {
      send(response, 200, {
        issuer,
        device_authorization_endpoint: `${issuer}/device`,
        token_endpoint: `${issuer}/token`,
      });
      return;
    }
    const body = new URLSearchParams(await readBody(request));
    if (request.url === '/issuer/device') {
      expect(body.get('client_id')).toBe('conformance-client');
      expect(body.get('code_challenge_method')).toBe('S256');
      observedChallenge = body.get('code_challenge') ?? '';
      send(response, 200, {
        device_code: 'conformance-device',
        user_code: 'TEST-CODE',
        verification_uri: `${issuer}/verify`,
        expires_in: 60,
        interval: 1,
      });
      return;
    }
    if (request.url === '/issuer/token') {
      if (body.get('grant_type') === 'refresh_token') {
        expect(body.get('refresh_token')).toBe('conformance-refresh');
        refreshCount += 1;
        send(response, 200, {
          access_token: 'conformance-refreshed',
          token_type: 'Bearer',
          expires_in: 300,
        });
        return;
      }
      observedVerifier = body.get('code_verifier') ?? '';
      pollCount += 1;
      if (pollCount === 1) {
        send(response, 400, { error: 'authorization_pending' });
        return;
      }
      send(response, 200, {
        access_token: 'conformance-access',
        refresh_token: 'conformance-refresh',
        token_type: 'Bearer',
        expires_in: 300,
      });
      return;
    }
    send(response, 404, { error: 'not_found' });
  }
});

function readBody(request: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    request.on('data', (chunk: Buffer) => chunks.push(chunk));
    request.on('end', () => {
      resolve(Buffer.concat(chunks).toString('utf8'));
    });
    request.on('error', reject);
  });
}

function send(response: ServerResponse, status: number, body: unknown): void {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(body));
}
