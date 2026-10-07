const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { resolve } = require('node:path');
const { test } = require('node:test');
const { parsePeersFile, peerLinkQuerySchemes } = require('@baukit/suite-client/peers');

test('requires the CommonJS peers export without runtime dependencies', () => {
  const entry = require.resolve('@baukit/suite-client/peers');
  assert.match(entry, /\/dist\/cjs\/peers\.js$/u);
  const sample = JSON.parse(
    readFileSync(resolve(__dirname, '../../../../fixtures/suite-events/v1/peers.json'), 'utf8'),
  );
  const peers = parsePeersFile(sample);
  assert.deepEqual(
    peers.map((peer) => peer.displayName),
    ['Alpha', 'Beta'],
  );
  assert.deepEqual(peerLinkQuerySchemes(peers, 'alpha'), ['beta']);
  assert.deepEqual(require.cache[entry].children, []);
  assert.throws(() => parsePeersFile({ schemaVersion: 1, peers: [null] }), TypeError);
});
