import {
  assertNamespace,
  isValidContentDigest,
  isValidInstant,
  isValidLogicalId,
  isValidNamespace,
  type PlannedNotification,
} from './validation.js';

export const OWNED_NOTIFICATION_DATA_KEY = 'baukitNotification';
export const OWNED_NOTIFICATION_MARKER_VERSION = 1;

const IDENTIFIER_PREFIX = 'baukit';
const IDENTIFIER_SEPARATOR = ':';

export interface OwnedNotificationMarker extends PlannedNotification {
  readonly namespace: string;
}

export function ownedNotificationIdentifier(namespace: string, logicalId: string): string {
  assertNamespace(namespace);
  return [IDENTIFIER_PREFIX, namespace, logicalId].join(IDENTIFIER_SEPARATOR);
}

export function encodeOwnedNotificationMarker(marker: OwnedNotificationMarker): string {
  return JSON.stringify({
    version: OWNED_NOTIFICATION_MARKER_VERSION,
    namespace: marker.namespace,
    logicalId: marker.logicalId,
    epochMilliseconds: marker.epochMilliseconds,
    contentDigest: marker.contentDigest,
  });
}

export function decodeOwnedNotificationMarker(value: unknown): OwnedNotificationMarker | null {
  if (typeof value !== 'string') {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return null;
  }
  const record = parsed as Record<string, unknown>;
  const { namespace, logicalId, epochMilliseconds, contentDigest } = record;
  if (
    record['version'] !== OWNED_NOTIFICATION_MARKER_VERSION ||
    !isValidNamespace(namespace) ||
    !isValidLogicalId(logicalId) ||
    !isValidInstant(epochMilliseconds) ||
    !isValidContentDigest(contentDigest)
  ) {
    return null;
  }
  return { namespace, logicalId, epochMilliseconds, contentDigest };
}

export function isOwnedBy(
  namespace: string,
  identifier: string,
  marker: OwnedNotificationMarker | null,
): marker is OwnedNotificationMarker {
  return (
    marker !== null &&
    marker.namespace === namespace &&
    identifier === ownedNotificationIdentifier(namespace, marker.logicalId)
  );
}
