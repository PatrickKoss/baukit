#!/usr/bin/env python3
"""List the Kani harnesses under rust/ with the bounds their doc comments state.

Prints a markdown table (plan ID, crate, harness, bounds). `--check` fails when a `#[kani::proof]`
has no doc comment with a `Bounds:` line. `--count CRATE` prints how many harnesses one crate has,
which the Kani gate compares with the number Kani verified.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROOF_ATTRIBUTE = re.compile(r"^\s*#\[kani::proof(?:_for_contract\([^)]*\))?\]")
ATTRIBUTE = re.compile(r"^\s*#\[")
DOC_LINE = re.compile(r"^\s*///\s?(?P<text>.*)$")
FUNCTION = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+(?P<name>\w+)")
CRATE_PATH_DEPTH = 2
BOUNDS = re.compile(r"\bBounds:\s*(?P<bounds>.+)$")
PLAN_ID = re.compile(r"^(?P<id>[KV]-\d+)\b")


@dataclass(frozen=True)
class Harness:
    crate: str
    path: Path
    name: str
    plan_id: str | None
    bounds: str | None


def crate_of(path: Path, root: Path) -> str:
    parts = path.relative_to(root).parts
    if len(parts) > CRATE_PATH_DEPTH and parts[0] == "crates":
        return parts[1]
    return parts[0]


def _doc_above(lines: list[str], index: int) -> list[str]:
    docs: list[str] = []
    cursor = index - 1
    while cursor >= 0:
        line = lines[cursor]
        doc = DOC_LINE.match(line)
        if doc:
            docs.insert(0, doc.group("text").strip())
        elif not ATTRIBUTE.match(line):
            break
        cursor -= 1
    return docs


def _function_below(lines: list[str], index: int) -> str:
    for line in lines[index + 1 :]:
        found = FUNCTION.match(line)
        if found:
            return found.group("name")
    return "?"


def harnesses_in(path: Path, root: Path) -> list[Harness]:
    lines = path.read_text(encoding="utf-8").splitlines()
    found = []
    for index, line in enumerate(lines):
        if not PROOF_ATTRIBUTE.match(line):
            continue
        doc = " ".join(_doc_above(lines, index))
        bounds = BOUNDS.search(doc)
        plan_id = PLAN_ID.match(doc)
        found.append(
            Harness(
                crate_of(path, root),
                path,
                _function_below(lines, index),
                plan_id.group("id") if plan_id else None,
                bounds.group("bounds").strip() if bounds else None,
            )
        )
    return found


def collect(root: Path) -> list[Harness]:
    harnesses = []
    for path in sorted(root.rglob("*.rs")):
        if "target" in path.relative_to(root).parts:
            continue
        harnesses.extend(harnesses_in(path, root))
    return harnesses


def table(harnesses: list[Harness]) -> str:
    rows = ["| Plan ID | Crate | Harness | Bounds |", "|---|---|---|---|"]
    for harness in harnesses:
        plan_id = harness.plan_id or "-"
        bounds = harness.bounds or "missing"
        rows.append(f"| {plan_id} | {harness.crate} | `{harness.name}` | {bounds} |")
    return "\n".join(rows)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check", action="store_true", help="fail on a harness without a Bounds: line"
    )
    parser.add_argument("--count", metavar="CRATE", help="print how many harnesses CRATE has")
    parser.add_argument("--root", type=Path, default=ROOT / "rust")
    args = parser.parse_args(argv)
    harnesses = collect(args.root)
    if args.count:
        print(sum(1 for harness in harnesses if harness.crate == args.count))
        return 0
    if not args.check:
        print(table(harnesses))
        return 0
    missing = [harness for harness in harnesses if harness.bounds is None]
    for harness in missing:
        print(
            f"ERROR: {harness.path}: harness {harness.name} has no `Bounds:` doc line",
            file=sys.stderr,
        )
    print(f"list-proofs: {len(harnesses)} harnesses, {len(missing)} without bounds")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
