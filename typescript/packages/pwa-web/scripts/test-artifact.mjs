import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const esm = await import('@baukit/pwa-web');
const require = createRequire(import.meta.url);
const commonjs = require('@baukit/pwa-web');
assert.equal(typeof esm.createFetchHandler, 'function');
assert.equal(typeof commonjs.createFetchHandler, 'function');

const artifactUrl = import.meta.resolve('@baukit/pwa-web/worker');
const source = await readFile(new URL(artifactUrl), 'utf8');
const workerGlobal = vm.createContext({ URL });

assert.equal('window' in workerGlobal, false);
assert.equal('document' in workerGlobal, false);
assert.equal('self' in workerGlobal, false);
vm.runInContext(source, workerGlobal, { filename: 'worker.js' });

const pwa = workerGlobal.BaukitPwa;
assert.equal(typeof pwa, 'object');
assert.equal(
  pwa.decideCacheStrategy(
    { url: 'https://app.example.test/api/v1/sync/pull' },
    {
      appOrigin: 'https://app.example.test',
      neverCachedPathPrefixes: ['/api/v1/sync'],
    },
  ),
  'network-only',
);
assert.equal(
  pwa.decideCacheStrategy(
    { url: 'https://app.example.test/_expo/static/js/web/app.js', destination: 'script' },
    { appOrigin: 'https://app.example.test', staticPathPrefixes: ['/_expo/static/'] },
  ),
  'cache-first',
);

const cache = new Map([['/offline.html', { body: 'offline' }]]);
const handler = pwa.createFetchHandler({
  appOrigin: 'https://app.example.test',
  navigationFallback: '/offline.html',
  ports: {
    fetch: () => Promise.reject(new Error('offline')),
    matchCache: (request) =>
      Promise.resolve(cache.get(typeof request === 'string' ? request : request.url)),
    putCache: () => Promise.resolve(),
    isCacheable: () => true,
    cloneResponse: (response) => response,
  },
});
assert.deepEqual(await handler({ url: 'https://app.example.test/plans', mode: 'navigate' }), {
  body: 'offline',
});

const cacheNames = [
  'notes-app-v1',
  'notes-app-v2',
  'notes-private-account-17',
  'notes-private-account-42',
];
const deleted = [];
const cleanup = (shouldDelete) =>
  pwa.cleanupCaches({
    ports: {
      listCacheNames: () => Promise.resolve(cacheNames),
      deleteCache: (name) => {
        deleted.push(name);
        return Promise.resolve(true);
      },
    },
    shouldDelete,
  });

const migration = await cleanup((name) => name.startsWith('notes-app-') && name !== 'notes-app-v2');
assert.equal(migration.matchedCount, 1);
assert.equal(migration.deletedCount, 1);
const identitySwitch = await cleanup((name) => name === 'notes-private-account-17');
assert.equal(identitySwitch.matchedCount, 1);
assert.equal(identitySwitch.deletedCount, 1);
assert.deepEqual(deleted, ['notes-app-v1', 'notes-private-account-17']);

for (const forbidden of ['node:', 'react', 'react-native', 'window.', 'document.']) {
  assert.equal(source.includes(forbidden), false, `worker artifact contains ${forbidden}`);
}

console.log('Worker artifact passed worker-like import, routing, offline, and cleanup checks.');
