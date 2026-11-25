/** Number of logical counter values available in one physical millisecond. */
export const HLC_COUNTERS_PER_MILLISECOND = 1_000;

/** Largest encoded timestamp JavaScript can represent exactly. */
export const MAX_HLC_TIMESTAMP = Number.MAX_SAFE_INTEGER;

/** Default bound on how far a remote wall time may lead the local physical clock. */
export const DEFAULT_HLC_MAX_DRIFT_MS = 24 * 60 * 60 * 1_000;

/** Wall time and logical counter of a clock. */
export type HlcStep = readonly [wallTimeMs: number, counter: number];

/** Options for {@link HybridLogicalClock.open}. */
export interface HybridLogicalClockOptions {
  /** How far a remote wall time may lead the physical clock. Defaults to 24 hours. */
  readonly maxDriftMs?: number;
}

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
  | 'empty_device_id'
  | 'remote_too_far_ahead'
  | 'invalid_max_drift';

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

/** Computes the state after a local event, without side effects. */
export function nowStep(state: HlcStep, physical: number): HlcStep {
  checkPhysical(physical);
  encodeHybridLogicalTimestamp(state[0], state[1]);
  const wall = Math.max(physical, state[0]);
  return rollOver(wall, physical > state[0] ? 0 : state[1] + 1);
}

/** Computes the state after receiving `remote`, rejecting remotes beyond `physical + maxDriftMs`. */
export function observeStep(
  state: HlcStep,
  physical: number,
  remote: number,
  maxDriftMs: number,
): HlcStep {
  const decoded = decodeHybridLogicalTimestamp(remote);
  checkPhysical(physical);
  encodeHybridLogicalTimestamp(state[0], state[1]);
  checkDrift(decoded.wallTimeMs, physical, maxDriftMs);
  const wall = Math.max(physical, state[0], decoded.wallTimeMs);
  let counter = 0;
  if (wall === state[0] && wall === decoded.wallTimeMs) {
    counter = Math.max(state[1], decoded.counter) + 1;
  } else if (wall === state[0]) {
    counter = state[1] + 1;
  } else if (wall === decoded.wallTimeMs) {
    counter = decoded.counter + 1;
  }
  return rollOver(wall, counter);
}

/** Rejects a client timestamp whose wall time leads `serverNowMs` by more than `maxDriftMs`. */
export function validateClientTimestamp(
  timestamp: number,
  serverNowMs: number,
  maxDriftMs: number = DEFAULT_HLC_MAX_DRIFT_MS,
): void {
  const { wallTimeMs } = decodeHybridLogicalTimestamp(timestamp);
  checkPhysical(serverNowMs);
  checkDrift(wallTimeMs, serverNowMs, maxDriftMs);
}

/** Hybrid logical clock with injected time and optional persistence. */
export class HybridLogicalClock {
  readonly #deviceId: string;
  readonly #physicalClock: HlcPhysicalClock;
  readonly #storage: HlcStorage | undefined;
  readonly #maxDriftMs: number;
  #state: HybridLogicalClockState;
  #queue: Promise<void> = Promise.resolve();

  private constructor(
    deviceId: string,
    physicalClock: HlcPhysicalClock,
    storage: HlcStorage | undefined,
    state: HybridLogicalClockState,
    maxDriftMs: number,
  ) {
    this.#deviceId = deviceId;
    this.#physicalClock = physicalClock;
    this.#storage = storage;
    this.#state = state;
    this.#maxDriftMs = maxDriftMs;
  }

  /**
   * Opens a clock from optional injected storage. Missing state starts at wall time zero.
   * Corrupt or foreign stored state starts at the current physical time, so the clock does
   * not reissue timestamps below those it handed out before.
   */
  public static async open(
    deviceId: string,
    storage?: HlcStorage,
    physicalClock: HlcPhysicalClock = Date.now,
    options: HybridLogicalClockOptions = {},
  ): Promise<HybridLogicalClock> {
    validateDeviceId(deviceId);
    const maxDriftMs = options.maxDriftMs ?? DEFAULT_HLC_MAX_DRIFT_MS;
    validateMaxDrift(maxDriftMs);
    const stored: unknown = storage ? await storage.get(HLC_STORAGE_KEY) : undefined;
    const state = initialState(stored, deviceId, physicalClock);
    return new HybridLogicalClock(deviceId, physicalClock, storage, state, maxDriftMs);
  }

  /** Returns the next local timestamp after committing its state. */
  public now(): Promise<number> {
    return this.#serialized(async () => {
      const physical = readPhysicalTime(this.#physicalClock);
      return this.#commit(nowStep(this.#step(), physical));
    });
  }

  /**
   * Observes a remote timestamp and returns a committed timestamp ordered after it. Rejects
   * a remote whose wall time leads the physical clock by more than the drift bound.
   */
  public observe(remoteTimestamp: number): Promise<number> {
    return this.#serialized(async () => {
      const physical = readPhysicalTime(this.#physicalClock);
      return this.#commit(observeStep(this.#step(), physical, remoteTimestamp, this.#maxDriftMs));
    });
  }

  /** Returns an immutable copy of the last committed state. */
  public snapshot(): HybridLogicalClockState {
    return freezeState(this.#state);
  }

  #step(): HlcStep {
    return [this.#state.wallTimeMs, this.#state.counter];
  }

  async #commit([wallTimeMs, counter]: HlcStep): Promise<number> {
    const timestamp = encodeHybridLogicalTimestamp(wallTimeMs, counter);
    const nextState = freezeState({ wallTimeMs, counter, deviceId: this.#deviceId });
    if (this.#storage) await this.#storage.set(HLC_STORAGE_KEY, nextState);
    this.#state = nextState;
    return timestamp;
  }

  /** Re-reads shared storage so another tab's advances are never reissued. */
  async #refresh(storage: HlcStorage): Promise<void> {
    const stored = parseState(await storage.get(HLC_STORAGE_KEY), this.#deviceId);
    if (stored && compareStates(stored, this.#state) > 0) this.#state = stored;
  }

  #serialized<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    const storage = this.#storage;
    if (!storage) return operation();

    const locked = async (): Promise<TResult> => {
      await this.#refresh(storage);
      return operation();
    };
    const locks = webLocks();
    const run = locks
      ? () => locks.request(`baukit-hlc:${this.#deviceId}`, locked)
      : locked;
    const result = this.#queue.then(run, run);
    this.#queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

interface WebLockManager {
  request<T>(name: string, callback: () => Promise<T>): Promise<T>;
}

function webLocks(): WebLockManager | undefined {
  const navigatorLike = (globalThis as { navigator?: { locks?: WebLockManager } }).navigator;
  return navigatorLike?.locks;
}

function compareStates(left: HybridLogicalClockState, right: HybridLogicalClockState): number {
  return left.wallTimeMs - right.wallTimeMs || left.counter - right.counter;
}

function initialState(
  stored: unknown,
  deviceId: string,
  physicalClock: HlcPhysicalClock,
): HybridLogicalClockState {
  if (stored === undefined || stored === null) {
    return freezeState({ wallTimeMs: 0, counter: 0, deviceId });
  }
  const parsed = parseState(stored, deviceId);
  if (parsed) return parsed;
  return freezeState({ wallTimeMs: readPhysicalTime(physicalClock), counter: 0, deviceId });
}

function rollOver(wall: number, counter: number): HlcStep {
  const step: HlcStep = counter >= HLC_COUNTERS_PER_MILLISECOND ? [wall + 1, 0] : [wall, counter];
  encodeHybridLogicalTimestamp(step[0], step[1]);
  return step;
}

function checkPhysical(physical: number): void {
  if (!Number.isSafeInteger(physical) || physical < 0) {
    throw new HybridLogicalClockError(
      'invalid_physical_clock',
      'Physical clock must return a non-negative safe integer',
    );
  }
}

function validateMaxDrift(maxDriftMs: number): void {
  if (!Number.isSafeInteger(maxDriftMs) || maxDriftMs < 0) {
    throw new HybridLogicalClockError('invalid_max_drift', 'HLC max drift must not be negative');
  }
}

function checkDrift(remoteWall: number, physical: number, maxDriftMs: number): void {
  validateMaxDrift(maxDriftMs);
  if (remoteWall > physical + maxDriftMs) {
    throw new HybridLogicalClockError(
      'remote_too_far_ahead',
      'Remote hybrid logical timestamp is too far ahead of the local clock',
    );
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
  checkPhysical(value);
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
