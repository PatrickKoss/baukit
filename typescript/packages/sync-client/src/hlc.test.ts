import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  compareHybridLogicalTimestamps,
  decodeHybridLogicalTimestamp,
  encodeHybridLogicalTimestamp,
  HybridLogicalClock,
  HybridLogicalClockError,
  MAX_HLC_TIMESTAMP,
  type HlcStorage,
  type HybridLogicalClockState,
} from './hlc.js';

interface EncodeVector {
  readonly wallTimeMs: number;
  readonly counter: number;
  readonly timestamp: number;
}

interface CompareVector {
  readonly left: number;
  readonly right: number;
  readonly ordering: -1 | 0 | 1;
}

interface MergeVector {
  readonly deviceId: string;
  readonly initial: HybridLogicalClockState;
  readonly physicalTimeMs: number;
  readonly remoteTimestamp: number;
  readonly expectedTimestamp: number;
  readonly expectedState: HybridLogicalClockState;
}

interface HlcVectors {
  readonly encode: readonly EncodeVector[];
  readonly compare: readonly CompareVector[];
  readonly merge: readonly MergeVector[];
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

class BlockingStorage extends MemoryStorage {
  readonly writes: HybridLogicalClockState[] = [];
  readonly firstWriteStarted: Promise<void>;
  #reportFirstWrite: () => void = () => undefined;
  #releaseFirstWrite: () => void = () => undefined;
  readonly #firstWriteGate: Promise<void>;

  public constructor() {
    super();
    this.firstWriteStarted = new Promise((resolve) => {
      this.#reportFirstWrite = resolve;
    });
    this.#firstWriteGate = new Promise((resolve) => {
      this.#releaseFirstWrite = resolve;
    });
  }

  public override async set(key: string, value: HybridLogicalClockState): Promise<void> {
    this.writes.push(structuredClone(value));
    if (this.writes.length === 1) {
      this.#reportFirstWrite();
      await this.#firstWriteGate;
    }
    await super.set(key, value);
  }

  public releaseFirstWrite(): void {
    this.#releaseFirstWrite();
  }
}

class FailingOnceStorage extends MemoryStorage {
  #failed = false;

  public override set(key: string, value: HybridLogicalClockState): Promise<void> {
    if (!this.#failed) {
      this.#failed = true;
      return Promise.reject(new Error('write failed'));
    }
    return super.set(key, value);
  }
}

const fixtureUrl = new URL('../../../../fixtures/hlc/vectors-v1.json', import.meta.url);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const vectors = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as HlcVectors;

describe('HybridLogicalClock', () => {
  it('advances while physical time stalls and moves backward', async () => {
    let physical = 1_000;
    const clock = await HybridLogicalClock.open('device-a', undefined, () => physical);

    const first = await clock.now();
    const stalled = await clock.now();
    physical = 900;
    const backward = await clock.now();

    expect(first).toBeLessThan(stalled);
    expect(stalled).toBeLessThan(backward);
    expect(decodeHybridLogicalTimestamp(backward)).toEqual({ wallTimeMs: 1_000, counter: 2 });
  });

  it('rolls the logical counter into the next millisecond', async () => {
    const storage = new MemoryStorage();
    storage.values.set('hlc-state', { wallTimeMs: 1_000, counter: 999, deviceId: 'device-a' });
    const clock = await HybridLogicalClock.open('device-a', storage, () => 1_000);

    await expect(clock.now()).resolves.toBe(1_001_001);
    expect(clock.snapshot()).toEqual({ wallTimeMs: 1_001, counter: 0, deviceId: 'device-a' });
  });

  it('restores matching state', async () => {
    const storage = new MemoryStorage();
    storage.values.set('hlc-state', { wallTimeMs: 2_000, counter: 7, deviceId: 'device-a' });
    const clock = await HybridLogicalClock.open('device-a', storage, () => 1_000);

    await expect(clock.now()).resolves.toBe(2_000_009);
    expect(clock.snapshot()).toEqual({ wallTimeMs: 2_000, counter: 8, deviceId: 'device-a' });
  });

  it.each([
    null,
    { wallTimeMs: -1, counter: 0, deviceId: 'device-a' },
    { wallTimeMs: 1_000, counter: 1_000, deviceId: 'device-a' },
    { wallTimeMs: Number.MAX_SAFE_INTEGER, counter: 0, deviceId: 'device-a' },
    { wallTimeMs: 1_000, counter: 2, deviceId: 'device-b' },
  ])('resets corrupt or foreign stored state %#', async (stored) => {
    const storage = new MemoryStorage();
    storage.values.set('hlc-state', stored);
    const clock = await HybridLogicalClock.open('device-a', storage, () => 10);

    await expect(clock.now()).resolves.toBe(10_001);
  });

  it('observes remote time and leaves state unchanged for invalid input', async () => {
    const clock = await HybridLogicalClock.open('device-a', undefined, () => 900);

    await expect(clock.observe(1_000_006)).resolves.toBe(1_000_007);
    const snapshot = clock.snapshot();
    await expect(clock.observe(0)).rejects.toMatchObject({ code: 'invalid_timestamp' });
    expect(clock.snapshot()).toEqual(snapshot);
    expect(compareHybridLogicalTimestamps(0, 1)).toBeNull();
  });

  it('serializes concurrent persisted calls', async () => {
    const storage = new BlockingStorage();
    const clock = await HybridLogicalClock.open('device-a', storage, () => 1_000);

    const first = clock.now();
    const second = clock.now();
    const observed = clock.observe(1_000_010);
    await storage.firstWriteStarted;
    expect(storage.writes).toHaveLength(1);
    storage.releaseFirstWrite();

    await expect(Promise.all([first, second, observed])).resolves.toEqual([
      1_000_001, 1_000_002, 1_000_011,
    ]);
    expect(storage.writes).toHaveLength(3);
    expect(clock.snapshot()).toEqual({ wallTimeMs: 1_000, counter: 10, deviceId: 'device-a' });
  });

  it('continues the persistence queue after a failed write', async () => {
    const storage = new FailingOnceStorage();
    const clock = await HybridLogicalClock.open('device-a', storage, () => 1_000);

    await expect(clock.now()).rejects.toThrow('write failed');
    expect(clock.snapshot()).toEqual({ wallTimeMs: 0, counter: 0, deviceId: 'device-a' });
    await expect(clock.now()).resolves.toBe(1_000_001);
  });

  it('supports the maximum encoded value and reports exhaustion', async () => {
    expect(encodeHybridLogicalTimestamp(9_007_199_254_740, 990)).toBe(MAX_HLC_TIMESTAMP);
    expect(decodeHybridLogicalTimestamp(MAX_HLC_TIMESTAMP)).toEqual({
      wallTimeMs: 9_007_199_254_740,
      counter: 990,
    });
    expect(() => encodeHybridLogicalTimestamp(9_007_199_254_740, 991)).toThrow(
      HybridLogicalClockError,
    );

    const storage = new MemoryStorage();
    storage.values.set('hlc-state', {
      wallTimeMs: 9_007_199_254_740,
      counter: 990,
      deviceId: 'device-a',
    });
    const clock = await HybridLogicalClock.open('device-a', storage, () => 0);
    await expect(clock.now()).rejects.toMatchObject({ code: 'exceeds_safe_integer' });
    expect(clock.snapshot()).toEqual({
      wallTimeMs: 9_007_199_254_740,
      counter: 990,
      deviceId: 'device-a',
    });
  });

  it('rejects a blank device id and invalid physical time with typed errors', async () => {
    await expect(HybridLogicalClock.open('   ')).rejects.toMatchObject({
      code: 'empty_device_id',
    });
    const clock = await HybridLogicalClock.open('device-a', undefined, () => -1);
    await expect(clock.now()).rejects.toMatchObject({ code: 'invalid_physical_clock' });
  });
});

describe('cross-runtime HLC vectors', () => {
  it('matches the Rust encoding, comparison, observation, and state corpus', async () => {
    for (const vector of vectors.encode) {
      expect(encodeHybridLogicalTimestamp(vector.wallTimeMs, vector.counter)).toBe(
        vector.timestamp,
      );
      expect(decodeHybridLogicalTimestamp(vector.timestamp)).toEqual({
        wallTimeMs: vector.wallTimeMs,
        counter: vector.counter,
      });
    }

    for (const vector of vectors.compare) {
      expect(compareHybridLogicalTimestamps(vector.left, vector.right)).toBe(vector.ordering);
    }

    for (const vector of vectors.merge) {
      const storage = new MemoryStorage();
      storage.values.set('hlc-state', vector.initial);
      const clock = await HybridLogicalClock.open(
        vector.deviceId,
        storage,
        () => vector.physicalTimeMs,
      );
      await expect(clock.observe(vector.remoteTimestamp)).resolves.toBe(vector.expectedTimestamp);
      expect(clock.snapshot()).toEqual(vector.expectedState);
    }
  });
});
