/** Number of logical counter values available in one physical millisecond. */
export const HLC_COUNTERS_PER_MILLISECOND = 1_000;

/** Largest encoded timestamp JavaScript can represent exactly. */
export const MAX_HLC_TIMESTAMP = Number.MAX_SAFE_INTEGER;

/** Key passed to an injected HLC store. */
export const HLC_STORAGE_KEY = 'hlc-state';

/** Serializable state needed to restore a hybrid logical clock. */
export type HybridLogicalClockState = Readonly<
  {
    wallTimeMs: number;
    counter: number;
    deviceId: string;
  } & Record<string, string | number>
>;

/** Async persistence supplied by a product storage adapter. */
export interface HlcStorage {
  get(key: string): Promise<unknown>;
  set(key: string, value: HybridLogicalClockState): Promise<void>;
}

/** Physical clock supplied by the host, usually `Date.now`. */
export type HlcPhysicalClock = () => number;

/** Stable validation and exhaustion error codes. */
export type HybridLogicalClockErrorCode =
  | 'invalid_component'
  | 'invalid_timestamp'
  | 'exceeds_safe_integer'
  | 'invalid_physical_clock'
  | 'empty_device_id';

/** Invalid HLC input or exhausted JavaScript-safe timestamp space. */
export class HybridLogicalClockError extends Error {
  public constructor(
    readonly code: HybridLogicalClockErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'HybridLogicalClockError';
  }
}

/** Encodes physical milliseconds and a logical counter into one timestamp. */
export function encodeHybridLogicalTimestamp(wallTimeMs: number, counter: number): number {
  if (
    !Number.isSafeInteger(wallTimeMs) ||
    wallTimeMs < 0 ||
    !Number.isInteger(counter) ||
    counter < 0 ||
    counter >= HLC_COUNTERS_PER_MILLISECOND
  ) {
    throw new HybridLogicalClockError(
      'invalid_component',
      'Invalid hybrid logical timestamp component',
    );
  }

  const encoded = wallTimeMs * HLC_COUNTERS_PER_MILLISECOND + counter + 1;
  if (!Number.isSafeInteger(encoded) || encoded < 1) {
    throw new HybridLogicalClockError(
      'exceeds_safe_integer',
      'Hybrid logical timestamp exceeds safe integer range',
    );
  }
  return encoded;
}

/** Decodes a positive JavaScript-safe timestamp into physical and logical parts. */
export function decodeHybridLogicalTimestamp(timestamp: number): Readonly<{
  wallTimeMs: number;
  counter: number;
}> {
  if (!isEncodedTimestamp(timestamp)) {
    throw new HybridLogicalClockError(
      'invalid_timestamp',
      'Invalid encoded hybrid logical timestamp',
    );
  }

  const zeroBased = timestamp - 1;
  return Object.freeze({
    wallTimeMs: Math.floor(zeroBased / HLC_COUNTERS_PER_MILLISECOND),
    counter: zeroBased % HLC_COUNTERS_PER_MILLISECOND,
  });
}

/** Compares two valid encoded timestamps, or returns `null` for invalid input. */
export function compareHybridLogicalTimestamps(left: number, right: number): -1 | 0 | 1 | null {
  if (!isEncodedTimestamp(left) || !isEncodedTimestamp(right)) return null;
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

/** Hybrid logical clock with injected time and optional persistence. */
export class HybridLogicalClock {
  readonly #deviceId: string;
  readonly #physicalClock: HlcPhysicalClock;
  readonly #storage: HlcStorage | undefined;
  #state: HybridLogicalClockState;
  #queue: Promise<void> = Promise.resolve();

  private constructor(
    deviceId: string,
    physicalClock: HlcPhysicalClock,
    storage: HlcStorage | undefined,
    state: HybridLogicalClockState,
  ) {
    this.#deviceId = deviceId;
    this.#physicalClock = physicalClock;
    this.#storage = storage;
    this.#state = state;
  }

  /** Opens a clock from optional injected storage. Invalid stored state resets to zero. */
  public static async open(
    deviceId: string,
    storage?: HlcStorage,
    physicalClock: HlcPhysicalClock = Date.now,
  ): Promise<HybridLogicalClock> {
    validateDeviceId(deviceId);
    const persisted = storage ? parseState(await storage.get(HLC_STORAGE_KEY), deviceId) : null;
    const state = persisted ?? freezeState({ wallTimeMs: 0, counter: 0, deviceId });
    return new HybridLogicalClock(deviceId, physicalClock, storage, state);
  }

  /** Returns the next local timestamp after committing its state. */
  public now(): Promise<number> {
    return this.#serialized(async () => {
      const physical = readPhysicalTime(this.#physicalClock);
      const wallTimeMs = Math.max(physical, this.#state.wallTimeMs);
      const counter = physical > this.#state.wallTimeMs ? 0 : this.#state.counter + 1;
      return this.#advance(wallTimeMs, counter);
    });
  }

  /** Observes a remote timestamp and returns a committed timestamp ordered after it. */
  public observe(remoteTimestamp: number): Promise<number> {
    return this.#serialized(async () => {
      const remote = decodeHybridLogicalTimestamp(remoteTimestamp);
      const physical = readPhysicalTime(this.#physicalClock);
      const wallTimeMs = Math.max(physical, this.#state.wallTimeMs, remote.wallTimeMs);
      let counter: number;
      if (wallTimeMs === this.#state.wallTimeMs && wallTimeMs === remote.wallTimeMs) {
        counter = Math.max(this.#state.counter, remote.counter) + 1;
      } else if (wallTimeMs === this.#state.wallTimeMs) {
        counter = this.#state.counter + 1;
      } else if (wallTimeMs === remote.wallTimeMs) {
        counter = remote.counter + 1;
      } else {
        counter = 0;
      }
      return this.#advance(wallTimeMs, counter);
    });
  }

  /** Returns an immutable copy of the last committed state. */
  public snapshot(): HybridLogicalClockState {
    return freezeState(this.#state);
  }

  async #advance(wallTimeMs: number, counter: number): Promise<number> {
    if (counter >= HLC_COUNTERS_PER_MILLISECOND) {
      wallTimeMs += 1;
      counter = 0;
    }

    const timestamp = encodeHybridLogicalTimestamp(wallTimeMs, counter);
    const nextState = freezeState({ wallTimeMs, counter, deviceId: this.#deviceId });
    if (this.#storage) await this.#storage.set(HLC_STORAGE_KEY, nextState);
    this.#state = nextState;
    return timestamp;
  }

  #serialized<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    if (!this.#storage) return operation();

    const result = this.#queue.then(operation, operation);
    this.#queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

function isEncodedTimestamp(timestamp: number): boolean {
  return Number.isSafeInteger(timestamp) && timestamp >= 1;
}

function parseState(value: unknown, deviceId: string): HybridLogicalClockState | null {
  if (typeof value !== 'object' || value === null) return null;
  if (!('wallTimeMs' in value) || !('counter' in value) || !('deviceId' in value)) return null;

  const candidate = value as {
    readonly wallTimeMs: unknown;
    readonly counter: unknown;
    readonly deviceId: unknown;
  };
  if (
    typeof candidate.wallTimeMs !== 'number' ||
    typeof candidate.counter !== 'number' ||
    candidate.deviceId !== deviceId
  ) {
    return null;
  }

  try {
    encodeHybridLogicalTimestamp(candidate.wallTimeMs, candidate.counter);
  } catch (error) {
    if (error instanceof HybridLogicalClockError) return null;
    throw error;
  }
  return freezeState({
    wallTimeMs: candidate.wallTimeMs,
    counter: candidate.counter,
    deviceId,
  });
}

function readPhysicalTime(clock: HlcPhysicalClock): number {
  const value = Math.floor(clock());
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new HybridLogicalClockError(
      'invalid_physical_clock',
      'Physical clock must return a non-negative safe integer',
    );
  }
  return value;
}

function validateDeviceId(deviceId: string): void {
  if (deviceId.trim().length === 0) {
    throw new HybridLogicalClockError('empty_device_id', 'HLC device id must not be empty');
  }
}

function freezeState(state: HybridLogicalClockState): HybridLogicalClockState {
  return Object.freeze({ ...state });
}
