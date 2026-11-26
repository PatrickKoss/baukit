import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  encodeHybridLogicalTimestamp,
  type HlcStep,
  HybridLogicalClockError,
  nowStep,
  observeStep,
} from './hlc.js';

interface VectorStep {
  op: 'now' | 'observe';
  physical: number;
  remote: number | null;
  expect: { state: HlcStep; timestamp: number } | { error: string };
}

interface VectorTrace {
  name: string;
  start: HlcStep;
  steps: VectorStep[];
}

const fixtureUrl = new URL('../../../../fixtures/sync/hlc-generated-v1.json', import.meta.url);
const fixture = JSON.parse(readFileSync(fixtureUrl, 'utf8')) as {
  maxDriftMs: number;
  traces: VectorTrace[];
};

function run(state: HlcStep, step: VectorStep, maxDriftMs: number): VectorStep['expect'] {
  try {
    const next =
      step.op === 'now'
        ? nowStep(state, step.physical)
        : observeStep(state, step.physical, step.remote ?? 0, maxDriftMs);
    return { state: next, timestamp: encodeHybridLogicalTimestamp(next[0], next[1]) };
  } catch (error) {
    if (error instanceof HybridLogicalClockError) return { error: error.code };
    throw error;
  }
}

describe('generated HLC vectors (D-01)', () => {
  it.each(fixture.traces.map((trace) => [trace.name, trace] as const))('%s', (_name, trace) => {
    let state = trace.start;
    for (const step of trace.steps) {
      const actual = run(state, step, fixture.maxDriftMs);
      expect(actual).toEqual(step.expect);
      if ('state' in actual) state = actual.state;
    }
  });
});
