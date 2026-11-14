"""Run the Flux convention policies offline.

Runs kyverno test, then kyverno apply on every rendered base and overlay.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path

import chart_contents

HERE = Path(__file__).resolve().parent
FLUX_API = re.compile(r"^apiVersion: (helm|source|kustomize)\.toolkit\.fluxcd\.io/", re.MULTILINE)
DOCUMENT_SEPARATOR = re.compile(r"^---[ \t]*$", re.MULTILINE)
COUNTED = ("pass", "fail", "error", "skip")

Runner = Callable[[list[str]], tuple[int, str]]


def run_command(command: list[str]) -> tuple[int, str]:
    result = chart_contents.helm_yaml.process_runner.run_process(
        command, capture_output=True, text=True
    )
    return result.returncode, result.stdout + result.stderr


@dataclass
class Tally:
    counts: dict[str, Counter] = field(default_factory=lambda: defaultdict(Counter))
    failures: list[str] = field(default_factory=list)

    def add(self, universe: str, report: dict) -> None:
        for result in report.get("results") or []:
            policy = result.get("policy", "?")
            outcome = result.get("result", "error")
            self.counts[policy][outcome] += 1
            if outcome in ("fail", "error"):
                for resource in result.get("resources") or [{}]:
                    kind, name = resource.get("kind"), resource.get("name")
                    ref = f"{kind}/{resource.get('namespace', '')}/{name}"
                    self.failures.append(
                        f"{universe}: {policy}: {ref}: {result.get('message', '')}"
                    )

    def table(self, policies: Iterable[str]) -> str:
        header = f"{'POLICY':<36} {'PASS':>5} {'FAIL':>5} {'ERROR':>5} {'SKIP':>5}"
        rows = [header]
        for policy in sorted(policies):
            counts = self.counts.get(policy, Counter())
            rows.append(f"{policy:<36} " + " ".join(f"{counts[key]:>5}" for key in COUNTED))
        return "\n".join(rows)

    @property
    def failed(self) -> bool:
        return bool(self.failures)


def flux_documents(text: str) -> str:
    documents = [doc.strip("\n") for doc in DOCUMENT_SEPARATOR.split(text) if FLUX_API.search(doc)]
    return "".join(f"---\n{doc}\n" for doc in documents)


def policy_names(policy_dir: Path) -> list[str]:
    names = []
    for policy in sorted(policy_dir.glob("*.yaml")):
        match = re.search(r"^  name: (\S+)$", policy.read_text(), re.MULTILINE)
        if match:
            names.append(match.group(1))
    return names


def universes(rendered: Path) -> list[Path]:
    return sorted(rendered.glob("base-*.yaml")) + sorted(rendered.glob("overlay-*.yaml"))


def apply_policies(
    rendered: Path, policy_dir: Path, run: Runner, work: Path
) -> tuple[Tally, list[str]]:
    tally = Tally()
    problems = []
    for universe in universes(rendered):
        flux = flux_documents(universe.read_text())
        if not flux:
            continue
        resources = work / universe.name
        resources.write_text(flux)
        code, output = run(
            [
                "kyverno",
                "apply",
                str(policy_dir),
                "--resource",
                str(resources),
                "--policy-report",
                "--output-format",
                "json",
            ]
        )
        start = output.find("{")
        try:
            report = json.JSONDecoder().raw_decode(output[start:])[0] if start >= 0 else None
        except json.JSONDecodeError:
            report = None
        if report is None:
            problems.append(
                f"{universe.stem}: kyverno apply exited {code} without a report:\n{output.strip()}"
            )
            continue
        tally.add(universe.stem, report)
        summary = report.get("summary") or {}
        if code != 0 and not (summary.get("fail") or summary.get("error")):
            problems.append(f"{universe.stem}: kyverno apply exited {code}:\n{output.strip()}")
    return tally, problems


def main(argv: list[str] | None = None, run: Runner = run_command) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--chart-cache", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--policies", type=Path, default=HERE / "policies")
    parser.add_argument("--tests", type=Path, default=HERE / "tests")
    args = parser.parse_args(argv)

    status = chart_contents.main(
        ["--rendered", str(args.rendered), "--chart-cache", str(args.chart_cache)]
    )

    code, output = run(["kyverno", "test", str(args.tests)])
    summary = re.search(r"^Test Summary: .*$", output, re.MULTILINE)
    print(summary.group(0) if summary else output.strip())
    if code != 0:
        print(output.strip(), file=sys.stderr)
        status = 1

    with tempfile.TemporaryDirectory() as work:
        tally, problems = apply_policies(args.rendered, args.policies, run, Path(work))
    table = tally.table(policy_names(args.policies))
    print(table)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(table + "\n" + "".join(f"FAIL {line}\n" for line in tally.failures))
    for line in tally.failures + problems:
        print(f"ERROR: {line}", file=sys.stderr)
    return 1 if status or tally.failed or problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
