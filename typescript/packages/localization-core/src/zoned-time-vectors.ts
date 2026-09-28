import {
  resolveZonedLocalTime,
  type FoldPolicy,
  type GapPolicy,
  type LocalTimeTransition,
  type ZonedLocalTimeCode,
  type ZonedLocalTimeResult,
} from './zoned-time.js';

export interface ZonedTimeVectorOutcome {
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly instant?: string;
  readonly error?: ZonedLocalTimeCode;
}

export interface ZonedTimeVectorCase {
  readonly name: string;
  readonly civilDate: string;
  readonly civilTime: string;
  readonly timeZone: string;
  readonly transition: LocalTimeTransition | null;
  readonly outcomes: readonly ZonedTimeVectorOutcome[];
}

export interface ZonedTimeVectorFixture {
  readonly version: number;
  readonly gapPolicies: readonly GapPolicy[];
  readonly foldPolicies: readonly FoldPolicy[];
  readonly cases: readonly ZonedTimeVectorCase[];
}

export type ZonedTimeVectorExpectation =
  | { readonly ok: false; readonly code: ZonedLocalTimeCode }
  | {
      readonly ok: true;
      readonly epochMilliseconds: number;
      readonly transition: LocalTimeTransition | null;
    };

/** One vector: `actual()` must deep-equal `expected` (`undefined` fields count as absent). */
export interface ZonedTimeVectorCheck {
  readonly label: string;
  readonly expected: ZonedTimeVectorExpectation;
  readonly actual: () => ZonedLocalTimeResult;
}

/** Expands `fixtures/zoned-time/vectors-v1.json` into one check per case and policy pair. */
export function zonedTimeVectorChecks(fixture: ZonedTimeVectorFixture): ZonedTimeVectorCheck[] {
  return fixture.cases.flatMap((entry) =>
    entry.outcomes.map((outcome) => ({
      label: `${entry.name} gap=${outcome.gap} fold=${outcome.fold}`,
      expected: expectedResult(entry, outcome),
      actual: () =>
        resolveZonedLocalTime({
          civilDate: entry.civilDate,
          civilTime: entry.civilTime,
          timeZone: entry.timeZone,
          gap: outcome.gap,
          fold: outcome.fold,
        }),
    })),
  );
}

function expectedResult(
  entry: ZonedTimeVectorCase,
  outcome: ZonedTimeVectorOutcome,
): ZonedTimeVectorExpectation {
  if (outcome.error !== undefined) {
    return { ok: false, code: outcome.error };
  }
  return {
    ok: true,
    epochMilliseconds: Date.parse(outcome.instant ?? ''),
    transition: entry.transition,
  };
}
