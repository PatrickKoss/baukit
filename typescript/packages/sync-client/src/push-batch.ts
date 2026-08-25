/**
 * One unsent local change, as baukit sees it.
 *
 * `entityType` and `entityId` identify the row the change targets. Baukit never
 * interprets either: entity names, payloads, and operations stay product-owned.
 */
export interface PushCandidate {
  /** Stable identity of this queued change, unique across the outbox. */
  changeId: string;
  entityType: string;
  entityId: string;
}

/** One entity's place in the batch, after coalescing and ordering. */
export interface RankedPushItem<T extends PushCandidate> {
  /** The newest queued change for this entity; its payload is the one to send. */
  change: T;
  /**
   * Every queued change this item settles, oldest first. One server outcome
   * clears all of them, because outcomes are keyed by entity, not by change.
   */
  coveredChangeIds: string[];
}

export interface RankPushBatchOptions<T extends PushCandidate> {
  /**
   * Dependency rank of an entity type. Lower ranks are sent first, so a parent
   * type must rank below every type that references it. Unknown types sort last.
   */
  rank: (entityType: string) => number;
  /** Largest number of entities one request may carry. */
  batchSize: number;
  /**
   * Entities whose required children are still unsent, or otherwise not yet
   * safe to send. Held-back entities are dropped from the batch and retried on
   * a later run, once their children have been accepted.
   */
  isHeldBack?: (change: T) => boolean;
}

/**
 * Orders a push batch parent-before-child and coalesces per entity.
 *
 * Queued changes are grouped by `entityType:entityId`. Each group keeps the
 * position of its first occurrence, so foreign-key order survives coalescing,
 * and carries the newest change so the request sends current data. Groups are
 * then sorted by dependency rank, ties broken by that first position, and the
 * batch is truncated to `batchSize`.
 *
 * A parent whose children are unsent is held back rather than reordered: the
 * server would otherwise see a complete parent while a required child is still
 * missing.
 */
export function rankPushBatch<T extends PushCandidate>(
  pending: readonly T[],
  { rank, batchSize, isHeldBack }: RankPushBatchOptions<T>,
): RankedPushItem<T>[] {
  const grouped = new Map<string, { change: T; coveredChangeIds: string[]; order: number }>();

  pending.forEach((change, order) => {
    const key = `${change.entityType}:${change.entityId}`;
    const existing = grouped.get(key);
    grouped.set(key, {
      change,
      coveredChangeIds: [...(existing?.coveredChangeIds ?? []), change.changeId],
      order: existing?.order ?? order,
    });
  });

  return [...grouped.values()]
    .filter(({ change }) => !isHeldBack?.(change))
    .sort(
      (left, right) =>
        rank(left.change.entityType) - rank(right.change.entityType) || left.order - right.order,
    )
    .slice(0, batchSize)
    .map(({ change, coveredChangeIds }) => ({ change, coveredChangeIds }));
}

/**
 * Builds a {@link RankPushBatchOptions.rank} function from an ordered list of
 * entity types. Types absent from the list sort after every listed type.
 */
export function dependencyRankByOrder(order: readonly string[]): (entityType: string) => number {
  const ranks = new Map(order.map((entityType, index) => [entityType, index]));
  return (entityType) => ranks.get(entityType) ?? order.length;
}
