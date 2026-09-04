import {
  compareHybridLogicalTimestamps,
  decodeHybridLogicalTimestamp,
  encodeHybridLogicalTimestamp,
  HybridLogicalClock,
  HybridLogicalClockError,
  HLC_COUNTERS_PER_MILLISECOND,
  HLC_STORAGE_KEY,
  MAX_HLC_TIMESTAMP,
  type HlcPhysicalClock,
  type HlcStorage,
  type HybridLogicalClockErrorCode,
  type HybridLogicalClockState,
} from '@baukit/sync-client/hlc';
import { HybridLogicalClock as RootHybridLogicalClock } from '@baukit/sync-client';

declare const storage: HlcStorage;
type LegacyJson =
  null | boolean | number | string | readonly LegacyJson[] | { readonly [key: string]: LegacyJson };
interface LegacyHlcStorage {
  get(key: string): Promise<unknown>;
  set(key: string, value: { readonly [key: string]: LegacyJson }): Promise<void>;
}
declare const legacyStorage: LegacyHlcStorage;
const physicalClock: HlcPhysicalClock = () => 1_000;
const rootClockConstructor: typeof HybridLogicalClock = RootHybridLogicalClock;
const opened: Promise<HybridLogicalClock> = HybridLogicalClock.open(
  'device-a',
  storage,
  physicalClock,
);
const ephemeral: Promise<HybridLogicalClock> = HybridLogicalClock.open(
  'device-a',
  undefined,
  physicalClock,
);
const legacyOpened: Promise<HybridLogicalClock> = HybridLogicalClock.open(
  'device-a',
  legacyStorage,
  physicalClock,
);
const encoded: number = encodeHybridLogicalTimestamp(1_000, 2);
const decoded: Readonly<{ wallTimeMs: number; counter: number }> =
  decodeHybridLogicalTimestamp(encoded);
const ordering: -1 | 0 | 1 | null = compareHybridLogicalTimestamps(encoded, encoded);
const errorCode: HybridLogicalClockErrorCode = new HybridLogicalClockError(
  'invalid_timestamp',
  'invalid',
).code;

declare const state: HybridLogicalClockState;

export {
  decoded,
  encoded,
  ephemeral,
  errorCode,
  HLC_COUNTERS_PER_MILLISECOND,
  HLC_STORAGE_KEY,
  MAX_HLC_TIMESTAMP,
  legacyOpened,
  opened,
  ordering,
  rootClockConstructor,
  state,
};
