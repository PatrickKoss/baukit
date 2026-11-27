"""Checks the admitted-request p99 of a load smoke run from its k6 CSV samples.

k6 times requests on the wall clock. When the clock steps forward (WSL resyncs in steps of
seconds), every request in flight gains the step. The `clock` scenario sleeps a fixed tick and
records how much longer the wall clock says it took; a lag above the tolerance is a step, and each
admitted sample that spans it is shortened by the step before the p99 is taken.
"""

from __future__ import annotations

import argparse
import csv
import gzip
import math
import sys
from dataclasses import dataclass
from pathlib import Path

ADMITTED_METRIC = "admitted_duration"
CLOCK_METRIC = "clock_lag"
CLOCK_TICK_MS = 100.0
STEP_TOLERANCE_MS = 250.0
PERCENTILE = 99.0
THRESHOLD_FAILED = 99
NO_SAMPLES = 2


@dataclass(frozen=True)
class Sample:
    ended_ms: float
    value_ms: float


@dataclass(frozen=True)
class Report:
    p99_ms: float
    raw_p99_ms: float
    samples: int
    corrected: int
    steps: tuple[Sample, ...]


def read_samples(path: Path) -> dict[str, list[Sample]]:
    wanted = {ADMITTED_METRIC: [], CLOCK_METRIC: []}
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        for row in csv.DictReader(handle):
            samples = wanted.get(row["metric_name"])
            if samples is not None:
                samples.append(Sample(float(row["timestamp"]), float(row["metric_value"])))
    return wanted


def clock_steps(lags: list[Sample]) -> tuple[Sample, ...]:
    return tuple(lag for lag in lags if lag.value_ms > STEP_TOLERANCE_MS)


def spans(sample: Sample, step: Sample) -> bool:
    stepped_at = step.ended_ms - CLOCK_TICK_MS / 2
    started = sample.ended_ms - sample.value_ms
    return started < stepped_at - step.value_ms and sample.ended_ms >= stepped_at


def corrected(sample: Sample, steps: tuple[Sample, ...]) -> float:
    return sample.value_ms - sum(step.value_ms for step in steps if spans(sample, step))


def percentile(values: list[float], rank: float) -> float:
    ordered = sorted(values)
    index = max(math.ceil(rank / 100 * len(ordered)) - 1, 0)
    return ordered[index]


def report(samples: dict[str, list[Sample]]) -> Report | None:
    admitted = samples[ADMITTED_METRIC]
    if not admitted:
        return None
    steps = clock_steps(samples[CLOCK_METRIC])
    durations = [corrected(sample, steps) for sample in admitted]
    changed = sum(
        1 for sample, value in zip(admitted, durations, strict=True) if value != sample.value_ms
    )
    return Report(
        p99_ms=percentile(durations, PERCENTILE),
        raw_p99_ms=percentile([sample.value_ms for sample in admitted], PERCENTILE),
        samples=len(admitted),
        corrected=changed,
        steps=steps,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples", type=Path)
    parser.add_argument("--p99-ms", type=float, required=True)
    arguments = parser.parse_args(argv)
    if not arguments.samples.is_file():
        print(f"admitted_duration: {arguments.samples} is missing", file=sys.stderr)
        return NO_SAMPLES
    result = report(read_samples(arguments.samples))
    if result is None:
        print("admitted_duration: no admitted samples", file=sys.stderr)
        return NO_SAMPLES
    passed = result.p99_ms < arguments.p99_ms
    print(
        f"admitted_duration p99={result.p99_ms:.1f}ms (raw {result.raw_p99_ms:.1f}ms) "
        f"target<{arguments.p99_ms:.0f}ms samples={result.samples} "
        f"clock_steps={len(result.steps)} corrected={result.corrected} "
        f"{'passed' if passed else 'FAILED'}"
    )
    for step in result.steps:
        print(f"  wall clock stepped {step.value_ms:.0f}ms at {step.ended_ms:.0f}")
    return 0 if passed else THRESHOLD_FAILED


if __name__ == "__main__":
    sys.exit(main())
