import { describe, expect, it } from 'vitest';

import { SyncPayloadCompatibilityError } from './error.js';
import {
  dependencyRankByOrder,
  rankPushBatch,
  validatePushOutcomeCoverage,
  type PushCandidate,
} from './push-batch.js';

// A three-level graph: a container owns groups, a group owns leaves.
const rank = dependencyRankByOrder(['container', 'group', 'leaf']);

function change(
  entityType: string,
  entityId: string,
  changeId = `${entityType}-${entityId}`,
): PushCandidate {
  return { changeId, entityType, entityId };
}

function entityIds(items: readonly { change: PushCandidate }[]): string[] {
  return items.map(({ change: candidate }) => `${candidate.entityType}:${candidate.entityId}`);
}

describe('dependencyRankByOrder', () => {
  it('ranks listed types by position and unknown types last', () => {
    expect(rank('container')).toBe(0);
    expect(rank('leaf')).toBe(2);
    expect(rank('unlisted')).toBe(3);
  });
});

describe('rankPushBatch', () => {
  it('orders a batch parent before child', () => {
    const pending = [change('leaf', 'l1'), change('container', 'c1'), change('group', 'g1')];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(entityIds(batch)).toEqual(['container:c1', 'group:g1', 'leaf:l1']);
  });

  it('keeps queue order within one entity type', () => {
    const pending = [change('leaf', 'l2'), change('leaf', 'l1'), change('leaf', 'l3')];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(entityIds(batch)).toEqual(['leaf:l2', 'leaf:l1', 'leaf:l3']);
  });

  it('sorts unknown entity types after every listed type', () => {
    const pending = [change('unlisted', 'u1'), change('leaf', 'l1')];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(entityIds(batch)).toEqual(['leaf:l1', 'unlisted:u1']);
  });

  it('coalesces repeated changes to one entity, sending the newest', () => {
    const pending = [
      change('leaf', 'l1', 'change-1'),
      change('leaf', 'l1', 'change-2'),
      change('leaf', 'l1', 'change-3'),
    ];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(batch).toHaveLength(1);
    expect(batch[0]?.change.changeId).toBe('change-3');
    expect(batch[0]?.coveredChangeIds).toEqual(['change-1', 'change-2', 'change-3']);
  });

  it('keeps the first occurrence position when coalescing, so parents stay ahead', () => {
    const pending = [
      change('container', 'c1', 'change-1'),
      change('leaf', 'l1', 'change-2'),
      change('container', 'c1', 'change-3'),
    ];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(entityIds(batch)).toEqual(['container:c1', 'leaf:l1']);
    expect(batch[0]?.change.changeId).toBe('change-3');
  });

  it('truncates to the batch size after ordering, keeping parents', () => {
    const pending = [
      change('leaf', 'l1'),
      change('leaf', 'l2'),
      change('container', 'c1'),
      change('group', 'g1'),
    ];

    const batch = rankPushBatch(pending, { rank, batchSize: 2 });

    expect(entityIds(batch)).toEqual(['container:c1', 'group:g1']);
  });

  it('holds back a parent whose children are unsent, and sends the children', () => {
    const pending = [
      change('container', 'c1'),
      change('group', 'g1'),
      change('leaf', 'l1'),
      change('leaf', 'l2'),
    ];
    const unsentChildren = new Set(['c1']);

    const batch = rankPushBatch(pending, {
      rank,
      batchSize: 10,
      isHeldBack: (candidate) =>
        candidate.entityType === 'container' && unsentChildren.has(candidate.entityId),
    });

    expect(entityIds(batch)).toEqual(['group:g1', 'leaf:l1', 'leaf:l2']);
  });

  it('sends a held-back parent on the next run once its children settled', () => {
    const pending = [change('container', 'c1'), change('leaf', 'l1')];
    const held = new Set(['c1']);
    const isHeldBack = (candidate: PushCandidate): boolean => held.has(candidate.entityId);

    const first = rankPushBatch(pending, { rank, batchSize: 10, isHeldBack });
    expect(entityIds(first)).toEqual(['leaf:l1']);

    held.clear();
    const second = rankPushBatch(pending, { rank, batchSize: 10, isHeldBack });
    expect(entityIds(second)).toEqual(['container:c1', 'leaf:l1']);
  });

  it('holds back every coalesced change for the held entity', () => {
    const pending = [change('container', 'c1', 'change-1'), change('container', 'c1', 'change-2')];

    const batch = rankPushBatch(pending, {
      rank,
      batchSize: 10,
      isHeldBack: (candidate) => candidate.entityId === 'c1',
    });

    expect(batch).toEqual([]);
  });

  it('counts held-back entities against neither the batch size nor the order', () => {
    const pending = [change('container', 'c1'), change('group', 'g1'), change('leaf', 'l1')];

    const batch = rankPushBatch(pending, {
      rank,
      batchSize: 2,
      isHeldBack: (candidate) => candidate.entityType === 'container',
    });

    expect(entityIds(batch)).toEqual(['group:g1', 'leaf:l1']);
  });

  it('returns an empty batch for an empty queue', () => {
    expect(rankPushBatch([], { rank, batchSize: 10 })).toEqual([]);
  });

  it('carries product-defined change fields through the batch', () => {
    interface ProductChange extends PushCandidate {
      payload: string;
    }
    const pending: ProductChange[] = [
      { changeId: 'change-1', entityType: 'leaf', entityId: 'l1', payload: 'first' },
      { changeId: 'change-2', entityType: 'leaf', entityId: 'l1', payload: 'second' },
    ];

    const batch = rankPushBatch(pending, { rank, batchSize: 10 });

    expect(batch[0]?.change.payload).toBe('second');
  });
});

describe('validatePushOutcomeCoverage', () => {
  const key = ({ entityType, entityId }: { entityType: string; entityId: string }): string =>
    `${entityType}:${entityId}`;

  it('returns a complete mix of accepted and rejected outcomes', () => {
    const submitted = [change('container', 'c1'), change('leaf', 'l1')];
    const outcomes = [
      { entityType: 'container', entityId: 'c1', result: 'accepted' },
      { entityType: 'leaf', entityId: 'l1', result: 'rejected' },
    ];

    expect(
      validatePushOutcomeCoverage(submitted, outcomes, {
        submittedKey: key,
        outcomeKey: key,
      }),
    ).toBe(outcomes);
  });

  it('rejects a partial outcome set before a caller acknowledges changes', () => {
    const acknowledged: string[] = [];
    const submitted = [change('container', 'c1'), change('leaf', 'l1')];
    const outcomes = [{ entityType: 'container', entityId: 'c1', result: 'accepted' }];

    expect(() => {
      const validated = validatePushOutcomeCoverage(submitted, outcomes, {
        submittedKey: key,
        outcomeKey: key,
      });
      acknowledged.push(...validated.map(key));
    }).toThrow(SyncPayloadCompatibilityError);
    expect(acknowledged).toEqual([]);
  });
});
