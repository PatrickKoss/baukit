import { NotificationPlanError } from './errors.js';

export const MAX_NAMESPACE_LENGTH = 64;
export const MAX_LOGICAL_ID_LENGTH = 128;
export const MAX_CONTENT_DIGEST_LENGTH = 128;

const NAMESPACE_PATTERN = /^[a-z0-9]+(?:[.-][a-z0-9]+)*$/;
const VISIBLE_ASCII_PATTERN = /^[!-~]+$/;

export interface PlannedNotification {
  readonly logicalId: string;
  readonly epochMilliseconds: number;
  readonly contentDigest: string;
}

export function isValidNamespace(namespace: unknown): namespace is string {
  return (
    typeof namespace === 'string' &&
    namespace.length <= MAX_NAMESPACE_LENGTH &&
    NAMESPACE_PATTERN.test(namespace)
  );
}

export function isValidLogicalId(logicalId: unknown): logicalId is string {
  return isVisibleAscii(logicalId, MAX_LOGICAL_ID_LENGTH);
}

export function isValidContentDigest(contentDigest: unknown): contentDigest is string {
  return isVisibleAscii(contentDigest, MAX_CONTENT_DIGEST_LENGTH);
}

export function isValidInstant(epochMilliseconds: unknown): epochMilliseconds is number {
  return (
    typeof epochMilliseconds === 'number' &&
    Number.isSafeInteger(epochMilliseconds) &&
    !Number.isNaN(new Date(epochMilliseconds).getTime())
  );
}

export function assertNamespace(namespace: unknown): asserts namespace is string {
  if (!isValidNamespace(namespace)) {
    throw new NotificationPlanError('invalid_namespace');
  }
}

export function assertIdentity(
  entry: { readonly logicalId: unknown; readonly contentDigest: unknown },
  seen: Set<string>,
): void {
  if (!isValidLogicalId(entry.logicalId)) {
    throw new NotificationPlanError('invalid_logical_id');
  }
  if (!isValidContentDigest(entry.contentDigest)) {
    throw new NotificationPlanError('invalid_content_digest', entry.logicalId);
  }
  if (seen.has(entry.logicalId)) {
    throw new NotificationPlanError('duplicate_logical_id', entry.logicalId);
  }
  seen.add(entry.logicalId);
}

export function assertPlannedNotifications(entries: readonly PlannedNotification[]): void {
  const seen = new Set<string>();
  for (const entry of entries) {
    assertIdentity(entry, seen);
    if (!isValidInstant(entry.epochMilliseconds)) {
      throw new NotificationPlanError('invalid_instant', entry.logicalId);
    }
  }
}

export function compareByInstant(left: PlannedNotification, right: PlannedNotification): number {
  return (
    left.epochMilliseconds - right.epochMilliseconds ||
    compareCodeUnits(left.logicalId, right.logicalId)
  );
}

export function compareCodeUnits(left: string, right: string): number {
  if (left === right) {
    return 0;
  }
  return left < right ? -1 : 1;
}

function isVisibleAscii(value: unknown, maxLength: number): value is string {
  return (
    typeof value === 'string' && value.length <= maxLength && VISIBLE_ASCII_PATTERN.test(value)
  );
}
