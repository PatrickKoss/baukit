#!/usr/bin/env python3
"""Check that every job holding an OIDC token is a gated, publish-only job.

A job with `id-token: write` must declare an `environment:`, must not install or build
(`pnpm install`, `pnpm build`, `npm ci`, a local `npm install`, `cargo build`),
and must depend, directly or through `needs`, on a job that runs
`git merge-base --is-ancestor`, so only commits on main reach the registries.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import yaml

BUILD_COMMAND = re.compile(
    r"\b(?:pnpm\s+(?:install|i|build)\b|npm\s+ci\b|cargo\s+build\b"
    r"|npm\s+(?:install|i)\b(?![^\n]*(?:--global|-g)\b))",
    re.MULTILINE,
)
ANCESTRY_CHECK = "merge-base --is-ancestor"


def _holds_id_token(permissions: object) -> bool:
    if permissions == "write-all":
        return True
    return isinstance(permissions, dict) and permissions.get("id-token") == "write"


def _needs(job: dict) -> list[str]:
    needs = job.get("needs", [])
    return [needs] if isinstance(needs, str) else list(needs)


def _runs(job: dict) -> list[str]:
    return [str(step["run"]) for step in job.get("steps", []) if "run" in step]


def _checks_ancestry(jobs: dict, name: str, seen: set[str]) -> bool:
    if name in seen or name not in jobs:
        return False
    seen.add(name)
    job = jobs[name]
    if any(ANCESTRY_CHECK in run for run in _runs(job)):
        return True
    return any(_checks_ancestry(jobs, parent, seen) for parent in _needs(job))


def job_violations(workflow: dict, source: str) -> list[str]:
    jobs = workflow.get("jobs") or {}
    default_permissions = workflow.get("permissions")
    violations = []
    for name, job in jobs.items():
        if not _holds_id_token(job.get("permissions", default_permissions)):
            continue
        where = f"{source}: job {name} holds id-token: write"
        if not job.get("environment"):
            violations.append(f"{where} without an environment")
        violations.extend(
            f"{where} and runs `{match.group(0).strip()}`"
            for run in _runs(job)
            for match in BUILD_COMMAND.finditer(run)
        )
        if not any(_checks_ancestry(jobs, parent, set()) for parent in _needs(job)):
            violations.append(f"{where} but needs no job that runs `git {ANCESTRY_CHECK}`")
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workflows", nargs="*", type=Path, default=[Path(".github/workflows")])
    args = parser.parse_args(argv)
    files = []
    for path in args.workflows:
        files.extend(sorted(path.glob("*.y*ml")) if path.is_dir() else [path])
    violations = []
    for path in files:
        workflow = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        violations.extend(job_violations(workflow, str(path)))
    for violation in violations:
        print(f"ERROR: {violation}", file=sys.stderr)
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
