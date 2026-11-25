import { describe, expect, it } from 'vitest';

import {
  compareHybridLogicalTimestamps,
  HybridLogicalClock,
  type HlcStorage,
  type HybridLogicalClockState,
} from './hlc.js';
import { rankPushBatch } from './push-batch.js';
import { validatePullPage } from './transport.js';

const RUNS = 200;
const SEED = 0x5eed;

class Random {
  #state: number;

  public constructor(seed: number) {
    this.#state = seed;
  }

  public below(bound: number): number {
    this.#state = (Math.imul(this.#state, 1_103_515_245) + 12_345) >>> 0;
    return this.#state % bound;
  }

  public pick<T>(values: readonly T[]): T {
    const value = values[this.below(values.length)];
    if (value === undefined) throw new Error('empty choice');
    return value;
  }
}

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

const ENTITY_PARTS = ['a', 'a:b', 'b', ':', '', '["x"]'];

describe('C-06 properties', () => {
  it('rankPushBatch covers each changeId once and never merges distinct entities', () => {
    const random = new Random(SEED);
    for (let run = 0; run < RUNS; run += 1) {
      const pending = Array.from({ length: random.below(12) }, (_, index) => ({
        changeId: `c${String(index)}`,
        entityType: random.pick(ENTITY_PARTS),
        entityId: random.pick(ENTITY_PARTS),
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
    }
  });

  it('validatePullPage accepts exactly the pages that advance or drain', () => {
    const random = new Random(SEED + 1);
    const compare = (left: number, right: number): number => left - right;
    for (let run = 0; run < RUNS; run += 1) {
      const current = random.below(5);
      const page = { nextCursor: random.below(5), hasMore: random.below(2) === 0 };
      const valid = page.nextCursor > current || (page.nextCursor === current && !page.hasMore);
      const check = (): unknown => validatePullPage(current, page, compare);
      if (valid) expect(check()).toBe(page);
      else expect(check).toThrow();
    }
  });

  it('two clocks on one storage issue strictly increasing timestamps', async () => {
    const random = new Random(SEED + 2);
    for (let run = 0; run < RUNS / 10; run += 1) {
      let physical = 1_000 + random.below(1_000);
      const storage = new MemoryStorage();
      const clocks = [
        await HybridLogicalClock.open('device-a', storage, () => physical),
        await HybridLogicalClock.open('device-a', storage, () => physical),
      ];
      let last: number | undefined;
      for (let step = 0; step < 30; step += 1) {
        physical += random.below(3) - 1;
        const clock = random.pick(clocks);
        const next =
          random.below(2) === 0
            ? await clock.now()
            : await clock.observe((physical + random.below(50)) * 1_000 + 1);
        if (last !== undefined) expect(compareHybridLogicalTimestamps(next, last)).toBe(1);
        last = next;
      }
    }
  });
});
