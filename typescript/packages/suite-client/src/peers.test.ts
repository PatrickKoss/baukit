import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parsePeersFile, peerLinkQuerySchemes, type PeerMetadata } from './peers.js';

interface Vector {
  readonly name: string;
  readonly valid: boolean;
}
interface PeerVector extends Vector {
  readonly patch?: Readonly<Record<string, unknown>>;
  readonly omit?: string;
}
interface FileVector extends Vector {
  readonly file: unknown;
}
interface Vectors {
  readonly basePeer: PeerMetadata;
  readonly peerCases: readonly PeerVector[];
  readonly fileCases: readonly FileVector[];
}
const fixtureRoot = new URL('../../../../fixtures/suite-events/v1/', import.meta.url);
const vectors = JSON.parse(
  readFileSync(new URL('peer-metadata.json', fixtureRoot), 'utf8'),
) as Vectors;
const sample: unknown = JSON.parse(readFileSync(new URL('peers.json', fixtureRoot), 'utf8'));

describe('shared Rust peer metadata vectors', () => {
  it.each(vectors.peerCases)('$name', (vector) => {
    const peer = Object.fromEntries(
      Object.entries({ ...vectors.basePeer, ...vector.patch }).filter(
        ([key]) => key !== vector.omit,
      ),
    );
    const file = { schemaVersion: 1, peers: [peer] };
    if (vector.valid) expect(parsePeersFile(file)).toEqual([peer]);
    else expect(() => parsePeersFile(file)).toThrow(TypeError);
  });

  it.each(vectors.fileCases)('$name', (vector) => {
    if (vector.valid) expect(parsePeersFile(vector.file)).toEqual([]);
    else expect(() => parsePeersFile(vector.file)).toThrow(TypeError);
  });
});

it('rejects duplicate ids, including when their schemes differ', () => {
  expect(() =>
    parsePeersFile({
      schemaVersion: 1,
      peers: [vectors.basePeer, { ...vectors.basePeer, scheme: 'other' }],
    }),
  ).toThrow(TypeError);
});

it('allows shared schemes and excludes the own app by id before deduplication', () => {
  const peers = parsePeersFile({
    schemaVersion: 1,
    peers: [
      { ...vectors.basePeer, id: 'own', scheme: 'shared' },
      { ...vectors.basePeer, id: 'gamma', scheme: 'z-last' },
      { ...vectors.basePeer, id: 'beta', scheme: 'shared' },
      { ...vectors.basePeer, id: 'delta', scheme: 'shared' },
      { ...vectors.basePeer, id: 'alpha', scheme: 'a-first' },
    ],
  });
  expect(peerLinkQuerySchemes(peers, 'own')).toEqual(['a-first', 'shared', 'z-last']);
  expect(peerLinkQuerySchemes(peers, 'missing')).toEqual(['a-first', 'shared', 'z-last']);
  expect(peers.map((peer) => peer.id)).toEqual(['own', 'gamma', 'beta', 'delta', 'alpha']);
});

it('returns no schemes for an empty file or only the own app', () => {
  expect(peerLinkQuerySchemes([], 'alpha')).toEqual([]);
  expect(peerLinkQuerySchemes([vectors.basePeer], 'alpha')).toEqual([]);
});

it('reads the neutral sample and returns the other app scheme', () => {
  expect(parsePeersFile(sample).map((peer) => peer.displayName)).toEqual(['Alpha', 'Beta']);
  expect(peerLinkQuerySchemes(parsePeersFile(sample), 'alpha')).toEqual(['beta']);
});

it('loads the ESM export with the same API', async () => {
  const esm = await import('@baukit/suite-client/peers');
  expect(import.meta.resolve('@baukit/suite-client/peers')).toMatch(/\/dist\/peers\.js$/u);
  expect(esm.parsePeersFile(sample)).toEqual(parsePeersFile(sample));
  expect(esm.peerLinkQuerySchemes(esm.parsePeersFile(sample), 'beta')).toEqual(['alpha']);
});
