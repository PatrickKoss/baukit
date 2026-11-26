import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import {
  compareHybridLogicalTimestamps,
  HybridLogicalClock,
  type HlcStorage,
  type HybridLogicalClockState,
} from './hlc.js';
import { rankPushBatch } from './push-batch.js';
import { validatePullPage } from './transport.js';

const MAX_PENDING = 12;
const CURSOR_RANGE = 4;
const CLOCK_STEPS = 30;
const MAX_REMOTE_LEAD_MS = 50;

class MemoryStorage implements HlcStorage {
  readonly values = new Map<string, unknown>();

  public get(key: string): Promise<unknown> {
    return Promise.resolve(this.values.get(key));
  }

  public set(key: string, value: HybridLogicalClockState): Promise<void> {
    this.values.set(key, structuredClone(value));
    return Promise.resolve();
  }
}

const entityPart = fc.constantFrom('a', 'a:b', 'b', ':', '', '["x"]');

const clockStep = fc.record({
  drift: fc.integer({ min: -1, max: 1 }),
  clock: fc.integer({ min: 0, max: 1 }),
  observe: fc.boolean(),
  lead: fc.integer({ min: 0, max: MAX_REMOTE_LEAD_MS }),
});

describe('C-06 properties', () => {
  it('rankPushBatch covers each changeId once and never merges distinct entities', () => {
    fc.assert(
      fc.property(
        fc.array(fc.tuple(entityPart, entityPart), { maxLength: MAX_PENDING }),
        (entities) => {
          const pending = entities.map(([entityType, entityId], index) => ({
            changeId: `c${String(index)}`,
            entityType,
            entityId,
          }));
          const ranked = rankPushBatch(pending, { rank: () => 0, batchSize: pending.length + 1 });
          const covered = ranked.flatMap((item) => item.coveredChangeIds);
          expect([...covered].sort()).toEqual(pending.map((change) => change.changeId).sort());
          for (const item of ranked) {
            for (const id of item.coveredChangeIds) {
              const source = pending.find((change) => change.changeId === id);
              expect([source?.entityType, source?.entityId]).toEqual([
                item.change.entityType,
                item.change.entityId,
              ]);
            }
          }
        },
      ),
    );
  });

  it('validatePullPage accepts exactly the pages that advance or drain', () => {
    const cursor = fc.integer({ min: 0, max: CURSOR_RANGE });
    const compare = (left: number, right: number): number => left - right;
    fc.assert(
      fc.property(cursor, cursor, fc.boolean(), (current, nextCursor, hasMore) => {
        const page = { nextCursor, hasMore };
        const valid = nextCursor > current || (nextCursor === current && !hasMore);
        const check = (): unknown => validatePullPage(current, page, compare);
        if (valid) expect(check()).toBe(page);
        else expect(check).toThrow();
      }),
    );
  });

  it('two clocks on one storage issue strictly increasing timestamps', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.integer({ min: 1_000, max: 2_000 }),
        fc.array(clockStep, { minLength: 1, maxLength: CLOCK_STEPS }),
        async (start, steps) => {
          let physical = start;
          const storage = new MemoryStorage();
          const clocks = [
            await HybridLogicalClock.open('device-a', storage, () => physical),
            await HybridLogicalClock.open('device-a', storage, () => physical),
          ];
          let last: number | undefined;
          for (const step of steps) {
            physical += step.drift;
            const clock = clocks[step.clock];
            if (clock === undefined) throw new Error('clock index out of range');
            const next = step.observe
              ? await clock.observe((physical + step.lead) * 1_000 + 1)
              : await clock.now();
            if (last !== undefined) expect(compareHybridLogicalTimestamps(next, last)).toBe(1);
            last = next;
          }
        },
      ),
    );
  });
});
