/** Embedded peer metadata. Products own the peers file. */
export interface PeerMetadata {
  readonly id: string;
  readonly scheme: string;
  readonly displayName: string;
  readonly emits: readonly string[];
  readonly accepts: readonly string[];
  readonly rewardModes: readonly ('native' | 'source_xp' | 'off')[];
}

const MAX_APP_ID_LENGTH = 64;
const PEER_FIELDS = ['id', 'scheme', 'displayName', 'emits', 'accepts', 'rewardModes'];

function requireFields(value: unknown, fields: readonly string[]): Record<string, unknown> {
  if (
    value === null ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    Object.keys(value).length !== fields.length ||
    !fields.every((field) => Object.hasOwn(value, field))
  ) {
    throw new TypeError('Invalid suite peers file fields');
  }
  return value as Record<string, unknown>;
}

function stringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item: unknown) => typeof item === 'string');
}

function parsePeer(value: unknown): PeerMetadata {
  const peer = requireFields(value, PEER_FIELDS);
  const { id, scheme, displayName, emits, accepts, rewardModes } = peer;
  if (
    typeof id !== 'string' ||
    id.length > MAX_APP_ID_LENGTH ||
    !/^[a-z]/u.test(id) ||
    /[^a-z0-9_]/u.test(id) ||
    typeof scheme !== 'string' ||
    !/^[a-z]/u.test(scheme) ||
    /[^a-z0-9+.-]/u.test(scheme) ||
    typeof displayName !== 'string' ||
    !stringArray(emits) ||
    !stringArray(accepts) ||
    !stringArray(rewardModes)
  ) {
    throw new TypeError('Invalid suite peer metadata');
  }
  const modes = rewardModes.map((mode) => {
    if (mode !== 'native' && mode !== 'source_xp' && mode !== 'off') {
      throw new TypeError('Invalid suite peer reward mode');
    }
    return mode;
  });
  return { id, scheme, displayName, emits: [...emits], accepts: [...accepts], rewardModes: modes };
}

/** Validates a parsed schema-version-1 peers file with the Rust registry rules. */
export function parsePeersFile(json: unknown): PeerMetadata[] {
  const file = requireFields(json, ['schemaVersion', 'peers']);
  if (file['schemaVersion'] !== 1 || !Array.isArray(file['peers'])) {
    throw new TypeError('Invalid suite peers file schema');
  }
  const peers = file['peers'].map(parsePeer);
  const ids = new Set(peers.map((peer) => peer.id));
  if (ids.size !== peers.length) {
    throw new TypeError('Duplicate suite peer id');
  }
  return peers;
}

/** Returns the other apps' query schemes, without duplicates, in lexical order. */
export function peerLinkQuerySchemes(peers: readonly PeerMetadata[], ownAppId: string): string[] {
  return [
    ...new Set(peers.filter((peer) => peer.id !== ownAppId).map((peer) => peer.scheme)),
  ].sort();
}
