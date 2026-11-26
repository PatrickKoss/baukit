import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { NodeTokenCache, type CachedTokenProfile } from './index.js';

const directories: string[] = [];
const DEAD_PID = 2_147_483_646;
const STALE_MS = 60_000;

afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true })));
});

async function cachePath(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-lock-'));
  directories.push(directory);
  return join(directory, 'tokens.json');
}

const PROFILE: CachedTokenProfile = {
  accessToken: 'access',
  tokenType: 'Bearer',
  issuedAt: 0,
  expiresAt: 1,
  issuer: 'https://issuer.example.test',
  clientId: 'cli',
  scopes: ['openid'],
};

function cache(path: string, now: () => number, alive = (pid: number) => pid !== DEAD_PID) {
  return new NodeTokenCache({
    namespace: 'test',
    path,
    now,
    sleep: () => Promise.resolve(),
    lockTimeoutMs: 5,
    lockRetryMs: 1,
    isProcessAlive: alive,
  });
}

describe('NodeTokenCache stale lock recovery', () => {
  it('removes a lock left by a dead process', async () => {
    const path = await cachePath();
    await writeFile(`${path}.lock`, JSON.stringify({ pid: DEAD_PID, createdAt: Date.now() }));
    await cache(path, Date.now).write('default', PROFILE);
    await expect(cache(path, Date.now).read()).resolves.toMatchObject({ accessToken: 'access' });
    await expect(stat(`${path}.lock`)).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('removes an unreadable lock once it is older than the stale age', async () => {
    const path = await cachePath();
    await writeFile(`${path}.lock`, 'garbage');
    const modified = (await stat(`${path}.lock`)).mtimeMs;
    await cache(path, () => modified + STALE_MS).write('default', PROFILE);
    await expect(readFile(path, 'utf8')).resolves.toContain('access');
  });

  it('waits for a fresh lock held by a live process', async () => {
    const path = await cachePath();
    const now = Date.now();
    await writeFile(`${path}.lock`, JSON.stringify({ pid: process.pid, createdAt: now }));
    let clock = now;
    const waiting = new NodeTokenCache({
      namespace: 'test',
      path,
      now: () => clock,
      sleep: (milliseconds) => {
        clock += milliseconds;
        return Promise.resolve();
      },
      lockTimeoutMs: 5,
      lockRetryMs: 1,
    });
    await expect(waiting.write('default', PROFILE)).rejects.toMatchObject({
      code: 'cache_lock_timeout',
    });
  });

  it('records its own pid in the lock while held', async () => {
    const path = await cachePath();
    const holder = await cache(path, Date.now).withLock(() =>
      readFile(`${path}.lock`, 'utf8').then((text) => JSON.parse(text) as { pid: number }),
    );
    expect(holder.pid).toBe(process.pid);
  });
});
