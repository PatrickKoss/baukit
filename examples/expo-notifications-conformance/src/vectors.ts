import {
  zonedTimeVectorChecks,
  type ZonedTimeVectorFixture,
} from "@baukit/localization-core/vectors";
import {
  notificationPlanVectorChecks,
  type NotificationPlanVectorFixture,
} from "@baukit/notifications-core/vectors";

import planVectors from "../../../fixtures/notifications/plan-vectors-v1.json";
import zonedVectors from "../../../fixtures/zoned-time/vectors-v1.json";
import { mismatch, type Mismatch } from "./check";

interface VectorCheck {
  readonly label: string;
  readonly expected: unknown;
  readonly actual: () => unknown;
}

export interface SuiteResult {
  readonly passed: number;
  readonly mismatches: readonly Mismatch[];
}

export interface VectorResults {
  readonly zonedTime: SuiteResult;
  readonly notificationPlan: SuiteResult;
}

export function runVectors(): VectorResults {
  const zoned = zonedTimeVectorChecks(zonedVectors as ZonedTimeVectorFixture);
  assertParsedInstants(zoned);
  return {
    zonedTime: runChecks(zoned),
    notificationPlan: runChecks(
      notificationPlanVectorChecks(
        planVectors as NotificationPlanVectorFixture,
      ),
    ),
  };
}

function runChecks(checks: readonly VectorCheck[]): SuiteResult {
  const mismatches: Mismatch[] = [];
  for (const check of checks) {
    const found = mismatch(check.label, evaluate(check), check.expected);
    if (found !== null) mismatches.push(found);
  }
  return { passed: checks.length - mismatches.length, mismatches };
}

function evaluate(check: VectorCheck): unknown {
  try {
    return check.actual();
  } catch (cause) {
    return {
      thrown:
        cause instanceof Error
          ? `${cause.name}: ${cause.message}`
          : String(cause),
    };
  }
}

// Expected instants come from Date.parse in this runtime; a parser that returned NaN on both
// sides would otherwise compare equal.
function assertParsedInstants(
  checks: ReturnType<typeof zonedTimeVectorChecks>,
): void {
  for (const check of checks) {
    if (
      check.expected.ok &&
      !Number.isFinite(check.expected.epochMilliseconds)
    ) {
      throw new Error(
        `${check.label}: Date.parse did not read the expected instant`,
      );
    }
  }
}
