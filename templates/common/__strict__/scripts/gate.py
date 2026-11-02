#!/usr/bin/env python3
"""Run gate steps as parallel lanes, select steps for --affected, and report timings.

The calling script owns the step commands. It lists its steps with
`<script> --list-steps` as lines of `name lane after mode tokens`, where
`after` and `tokens` are comma lists or `-`, and `mode` is `always` (also a
fast check under --affected), `selected` (runs when the full gate runs or a
token selects it), or `affected` (runs only under --affected). This runner
calls `<script> --step <name>` for each selected step.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

MODES = ("always", "selected", "affected")
DEFAULT_BUDGET_MINUTES = {"full": 30, "affected": 15}
DEFAULT_WORKER_MEMORY_MB = 1024
LIGHT_LANES = frozenset({"fast"})
FAILURE_TAIL_LINES = 40
GENERIC_SHARED = (
    "**/Cargo.lock",
    "**/pnpm-lock.yaml",
    "**/package-lock.json",
    "**/package.json",
    "**/pnpm-workspace.yaml",
    "**/rust-toolchain",
    "**/rust-toolchain.toml",
    "**/.cargo/**",
    "**/migrations/**",
    "baukit.toml",
    "limits.json",
    "scripts/**",
    "Makefile",
    "mise.toml",
    ".github/**",
    "compose.yaml",
)


@dataclass(frozen=True)
class Step:
    name: str
    lane: str
    after: tuple[str, ...]
    mode: str
    tokens: frozenset[str]


@dataclass
class Selection:
    full: bool
    reasons: list[str]
    tokens: set[str] = field(default_factory=set)


@dataclass(frozen=True)
class PlannedStep:
    step: Step
    reason: str


def split_list(value: str) -> tuple[str, ...]:
    return () if value == "-" else tuple(item for item in value.split(",") if item)


def parse_steps(text: str) -> list[Step]:
    steps = []
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 5:
            raise ValueError(f"step line {number} needs name, lane, after, mode and tokens: {line!r}")
        name, lane, after, mode, tokens = fields
        if mode not in MODES:
            raise ValueError(f"step {name} has unknown mode {mode}")
        steps.append(Step(name, lane, split_list(after), mode, frozenset({name, lane, *split_list(tokens)})))
    names = {step.name for step in steps}
    if len(names) != len(steps):
        raise ValueError("step names must be unique")
    for step in steps:
        unknown = [dependency for dependency in step.after if dependency not in names]
        if unknown:
            raise ValueError(f"step {step.name} runs after unknown steps: {', '.join(unknown)}")
    return steps


def glob_pattern(glob: str) -> re.Pattern[str]:
    parts = []
    index = 0
    while index < len(glob):
        if glob.startswith("**/", index):
            parts.append("(?:.*/)?")
            index += 3
        elif glob.startswith("**", index):
            parts.append(".*")
            index += 2
        elif glob[index] == "*":
            parts.append("[^/]*")
            index += 1
        elif glob[index] == "?":
            parts.append("[^/]")
            index += 1
        else:
            parts.append(re.escape(glob[index]))
            index += 1
    return re.compile("".join(parts))


def matches(glob: str, path: str) -> bool:
    return glob_pattern(glob).fullmatch(path) is not None


def base_token(token: str) -> str:
    return token.partition(":")[0]


def select(changed: list[str], affected: dict[str, list[str]], shared: list[str]) -> Selection:
    if not changed:
        return Selection(False, ["no changed files: fast checks only"])
    tokens: set[str] = set()
    reasons = []
    for path in changed:
        shared_glob = next((glob for glob in shared if matches(glob, path)), None)
        if shared_glob is not None:
            return Selection(True, [f"{path} matches shared path {shared_glob}: full gate"])
        globs = [glob for glob in affected if matches(glob, path)]
        if not globs:
            return Selection(True, [f"{path} matches no quality.affected glob: full gate"])
        for glob in globs:
            tokens.update(affected[glob])
            chosen = ", ".join(affected[glob]) or "fast checks only"
            reasons.append(f"{path} matches {glob}: {chosen}")
    return Selection(False, reasons, tokens)


def unknown_tokens(affected: dict[str, list[str]], steps: list[Step]) -> list[str]:
    known = set().union(*(step.tokens for step in steps)) if steps else set()
    return sorted({token for tokens in affected.values() for token in tokens if base_token(token) not in known})


def plan(steps: list[Step], selection: Selection) -> list[PlannedStep]:
    if selection.full:
        return [PlannedStep(step, "full gate") for step in steps if step.mode != "affected"]
    wanted = {base_token(token) for token in selection.tokens}
    planned = []
    for step in steps:
        if step.mode in ("always", "affected"):
            planned.append(PlannedStep(step, "fast check"))
            continue
        hits = sorted(step.tokens & wanted)
        if hits:
            planned.append(PlannedStep(step, f"selected by {', '.join(hits)}"))
    return planned


def workspace_members(metadata: dict, repository_root: Path) -> tuple[dict[str, str], dict[str, set[str]]]:
    members = {}
    for package in metadata["packages"]:
        if package["id"] not in metadata["workspace_members"]:
            continue
        directory = Path(package["manifest_path"]).resolve().parent
        members[package["name"]] = directory.relative_to(repository_root.resolve()).as_posix()
    dependents: dict[str, set[str]] = {name: set() for name in members}
    for package in metadata["packages"]:
        if package["name"] not in members:
            continue
        for dependency in package["dependencies"]:
            if dependency["name"] in members and dependency.get("path"):
                dependents[dependency["name"]].add(package["name"])
    return members, dependents


def affected_packages(
    changed: list[str], workspace: str, members: dict[str, str], dependents: dict[str, set[str]]
) -> list[str] | None:
    """Return the changed workspace packages and their reverse dependencies, or None for all."""
    chosen: set[str] = set()
    for path in changed:
        if not (path + "/").startswith(workspace.rstrip("/") + "/"):
            continue
        owners = [name for name, directory in members.items() if path.startswith(directory + "/")]
        if not owners:
            return None
        chosen.add(max(owners, key=lambda name: len(members[name])))
    pending = list(chosen)
    while pending:
        for dependent in dependents.get(pending.pop(), ()):
            if dependent not in chosen:
                chosen.add(dependent)
                pending.append(dependent)
    return sorted(chosen)


def total_memory_mb(meminfo: Path = Path("/proc/meminfo")) -> int | None:
    try:
        text = meminfo.read_text()
    except OSError:
        return None
    match = re.search(r"^MemTotal:\s+(\d+) kB", text, re.MULTILINE)
    return int(match[1]) // 1024 if match else None


def split_workers(cores: int, lanes: int, memory_mb: int | None, worker_memory_mb: int) -> int:
    lanes = max(1, lanes)
    share = max(1, cores // lanes)
    if memory_mb is not None and worker_memory_mb > 0:
        share = min(share, max(1, memory_mb // (worker_memory_mb * lanes)))
    return share


def heavy_lanes(planned: list[PlannedStep]) -> list[str]:
    lanes = []
    for item in planned:
        if item.step.lane not in LIGHT_LANES and item.step.lane not in lanes:
            lanes.append(item.step.lane)
    return lanes


def budget_report(
    timings: dict[str, int], lanes: dict[str, list[str]], lane_seconds: dict[str, int],
    wall: int, budget_minutes: int, mode: str,
) -> list[str]:
    lines = ["gate: step timings, slowest first"]
    width = max((len(name) for name in timings), default=4)
    for name, seconds in sorted(timings.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"  {name.ljust(width)}  {seconds:>6} s")
    for lane, seconds in sorted(lane_seconds.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"gate: lane {lane} took {seconds} s")
    budget = budget_minutes * 60
    if wall <= budget:
        lines.append(f"gate: {wall} s is within the {budget_minutes} minute {mode} budget")
        return lines
    lines.append(f"gate: warning: {wall} s is over the {budget_minutes} minute {mode} budget by {wall - budget} s")
    slowest = max(lane_seconds, key=lambda lane: lane_seconds[lane], default=None)
    if slowest is not None:
        over = sorted(lanes.get(slowest, []), key=lambda name: -timings.get(name, 0))
        steps = ", ".join(f"{name} {timings.get(name, 0)} s" for name in over)
        lines.append(f"gate: warning: lane {slowest} set the wall time: {steps}")
    return lines


def protected_spec_violations(spec_directory: Path) -> list[str]:
    violations = []
    for spec in sorted(spec_directory.glob("qa-*.spec.ts")):
        if "@logic" in spec.read_text(encoding="utf-8"):
            violations.append(f"{spec}: protected qa specs run on every project; remove @logic")
    return violations


def changed_files(base: str) -> list[str]:
    merge_base = git("merge-base", base, "HEAD") or base
    tracked = git("diff", "--name-only", merge_base).splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").splitlines()
    return sorted(set(tracked) | set(untracked))


def git(*arguments: str) -> str:
    result = subprocess.run(["git", *arguments], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise SystemExit(f"gate: git {' '.join(arguments)} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def load_settings(path: Path) -> dict:
    with path.open("rb") as source:
        return tomllib.load(source)


def shared_paths(settings: dict) -> list[str]:
    quality = settings.get("quality", {})
    openapi = settings.get("openapi", {})
    paths = list(GENERIC_SHARED)
    if "schema" in openapi:
        paths.append(openapi["schema"])
        paths.append(str(Path(openapi["schema"]).parent / "mcp-tools.json"))
    paths.extend(openapi.get("consumers", []))
    paths.extend(quality.get("shared", []))
    return paths


def budget_minutes(settings: dict, mode: str) -> int:
    configured = settings.get("quality", {}).get("gate_budget_minutes", {})
    return int(configured.get(mode, DEFAULT_BUDGET_MINUTES[mode]))


class Runner:
    def __init__(self, planned: list[PlannedStep], command: list[str], log_directory: Path, environment: dict[str, str]):
        self.planned = planned
        self.command = command
        self.log_directory = log_directory
        self.environment = environment
        self.names = {item.step.name for item in planned}
        self.done = {item.step.name: threading.Event() for item in planned}
        self.status: dict[str, str] = {}
        self.timings: dict[str, int] = {}
        self.lane_seconds: dict[str, int] = {}
        self.lock = threading.Lock()
        self.timings_file = log_directory / "timings.txt"

    def lanes(self) -> dict[str, list[Step]]:
        lanes: dict[str, list[Step]] = {}
        for item in self.planned:
            lanes.setdefault(item.step.lane, []).append(item.step)
        return lanes

    def record(self, name: str, seconds: int, status: str) -> None:
        with self.lock:
            self.timings[name] = seconds
            self.status[name] = status
            with self.timings_file.open("a") as output:
                output.write(f"{name} {seconds}\n")
        self.done[name].set()

    def blocked_by(self, step: Step) -> str | None:
        for dependency in step.after:
            if dependency not in self.names:
                continue
            self.done[dependency].wait()
            if self.status[dependency] != "passed":
                return dependency
        return None

    def run_lane(self, lane: str, steps: list[Step]) -> None:
        log = self.log_directory / f"{lane}.log"
        started = time.monotonic()
        failed = False
        for step in steps:
            if failed:
                self.status[step.name] = "not run"
                self.done[step.name].set()
                continue
            dependency = self.blocked_by(step)
            if dependency is not None:
                print(f"gate: [{lane}] {step.name} not run because {dependency} did not pass", flush=True)
                self.status[step.name] = f"blocked by {dependency}"
                self.done[step.name].set()
                failed = True
                continue
            failed = not self.run_step(lane, step, log)
        self.lane_seconds[lane] = round(time.monotonic() - started)

    def run_step(self, lane: str, step: Step, log: Path) -> bool:
        print(f"gate: [{lane}] {step.name} started", flush=True)
        started = time.monotonic()
        with log.open("a") as output:
            output.write(f"\n==> {step.name}\n")
            output.flush()
            result = subprocess.run(
                [*self.command, step.name], stdout=output, stderr=subprocess.STDOUT,
                env=self.environment, check=False,
            )
        seconds = round(time.monotonic() - started)
        passed = result.returncode == 0
        self.record(step.name, seconds, "passed" if passed else f"failed with exit {result.returncode}")
        if passed:
            print(f"gate: [{lane}] {step.name} passed in {seconds} s", flush=True)
            return True
        print(f"gate: [{lane}] {step.name} failed with exit {result.returncode} in {seconds} s; log {log}", flush=True)
        tail = log.read_text(errors="replace").splitlines()[-FAILURE_TAIL_LINES:]
        print("\n".join(f"  {line}" for line in tail), flush=True)
        return False

    def run(self) -> list[str]:
        self.timings_file.write_text("")
        threads = [
            threading.Thread(target=self.run_lane, args=(lane, steps), name=lane)
            for lane, steps in self.lanes().items()
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        failures = []
        for lane, steps in self.lanes().items():
            broken = [f"{step.name} {self.status[step.name]}" for step in steps if self.status.get(step.name) != "passed"]
            if broken:
                failures.append(f"{lane} ({'; '.join(broken)})")
        with self.timings_file.open("a") as output:
            for lane, seconds in self.lane_seconds.items():
                output.write(f"lane:{lane} {seconds}\n")
        return failures


def print_plan(mode: str, base: str | None, selection: Selection, planned: list[PlannedStep], extras: list[str]) -> None:
    print(f"gate: mode {mode}" + (f" against {base}" if base else ""))
    for reason in selection.reasons:
        print(f"gate: {reason}")
    for extra in extras:
        print(f"gate: {extra}")
    print("gate: steps")
    for item in planned:
        print(f"gate:   {item.step.lane}/{item.step.name} ({item.reason})")


def cache_environment(environment: dict[str, str]) -> list[str]:
    home = Path.home()
    environment.setdefault("CARGO_HOME", str(home / ".cargo"))
    environment.setdefault("GRADLE_USER_HOME", str(home / ".gradle"))
    lines = [f"CARGO_HOME {environment['CARGO_HOME']}", f"GRADLE_USER_HOME {environment['GRADLE_USER_HOME']}"]
    if "RUSTC_WRAPPER" not in environment and shutil.which("sccache"):
        environment["RUSTC_WRAPPER"] = "sccache"
    wrapper = environment.get("RUSTC_WRAPPER", "")
    lines.append("sccache on" if Path(wrapper).name == "sccache" else "sccache off (not on PATH)")
    return [f"cache: {line}" for line in lines]


def worker_environment(environment: dict[str, str], workers: int) -> None:
    for name in (
        "BAUKIT_GATE_WORKERS", "CARGO_BUILD_JOBS", "NEXTEST_TEST_THREADS", "RUST_TEST_THREADS",
        "VITEST_MAX_WORKERS", "E2E_WORKERS",
    ):
        environment[name] = str(workers)


def run_command(arguments: argparse.Namespace) -> int:
    script = shlex.split(arguments.script)
    listing = subprocess.run([*script, "--list-steps"], capture_output=True, text=True, check=True).stdout
    steps = parse_steps(listing)
    settings = load_settings(Path(arguments.settings))
    mode = "affected" if arguments.affected else "full"
    if arguments.affected:
        changed = changed_files(arguments.affected)
        affected = settings.get("quality", {}).get("affected", {})
        unknown = unknown_tokens(affected, steps)
        if unknown:
            print(f"gate: quality.affected names unknown lanes or steps: {', '.join(unknown)}", file=sys.stderr)
            return 2
        selection = select(changed, affected, shared_paths(settings))
    else:
        changed = []
        selection = Selection(True, ["full gate requested"])
    if selection.full:
        mode = "full"
    planned = plan(steps, selection)
    environment = dict(os.environ)
    environment["BAUKIT_GATE_MODE"] = mode
    environment["BAUKIT_GATE_BASE"] = arguments.affected or ""
    environment["BAUKIT_GATE_TOKENS"] = " ".join(sorted(selection.tokens))
    extras = []
    if arguments.cargo_workspace and mode == "affected" and "backend" in {base_token(t) for t in selection.tokens}:
        manifest = Path(arguments.cargo_workspace)
        metadata = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--manifest-path", str(manifest)],
            capture_output=True, text=True, check=True,
        ).stdout)
        members, dependents = workspace_members(metadata, Path.cwd())
        packages = affected_packages(changed, manifest.parent.as_posix(), members, dependents)
        environment["BAUKIT_GATE_RUST_PACKAGES"] = " ".join(packages or [])
        extras.append("rust packages: " + (", ".join(packages) if packages else "all"))
    else:
        environment["BAUKIT_GATE_RUST_PACKAGES"] = ""
    cores = int(os.environ.get("BAUKIT_GATE_CORES") or os.cpu_count() or 1)
    lanes = heavy_lanes(planned)
    worker_memory = int(os.environ.get("BAUKIT_GATE_WORKER_MEMORY_MB", DEFAULT_WORKER_MEMORY_MB))
    workers = split_workers(cores, len(lanes), total_memory_mb(), worker_memory)
    worker_environment(environment, workers)
    extras.append(f"workers: {cores} cores across {len(lanes)} lanes ({', '.join(lanes) or 'none'}): {workers} per lane")
    extras.extend(cache_environment(environment))
    print_plan(mode, arguments.affected, selection, planned, extras)
    if arguments.plan:
        return 0
    log_directory = Path(arguments.log_dir).resolve()
    log_directory.mkdir(parents=True, exist_ok=True)
    for stale in log_directory.glob("*.log"):
        stale.unlink()
    environment["BAUKIT_GATE_LOG_DIR"] = str(log_directory)
    started = time.monotonic()
    runner = Runner(planned, [*script, "--step"], log_directory, environment)
    failures = runner.run()
    wall = round(time.monotonic() - started)
    with runner.timings_file.open("a") as output:
        output.write(f"total {wall}\n")
    lane_steps = {lane: [step.name for step in steps] for lane, steps in runner.lanes().items()}
    report = budget_report(runner.timings, lane_steps, runner.lane_seconds, wall, budget_minutes(settings, mode), mode)
    print("\n".join(report))
    print(f"gate: timings in {runner.timings_file}; lane logs in {log_directory}")
    if failures:
        print("gate: failed lanes: " + ", ".join(failures))
        return 1
    print(f"gate: {arguments.name} passed in {wall} s")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("--script", required=True)
    run.add_argument("--settings", required=True)
    run.add_argument("--log-dir", required=True)
    run.add_argument("--name", default="quality gate")
    run.add_argument("--affected", metavar="BASE")
    run.add_argument("--cargo-workspace")
    run.add_argument("--plan", action="store_true")
    specs = commands.add_parser("check-specs")
    specs.add_argument("directory")
    arguments = parser.parse_args(argv)
    if arguments.command == "check-specs":
        violations = protected_spec_violations(Path(arguments.directory))
        print("\n".join(violations) or "gate: protected qa specs carry no @logic tag")
        return int(bool(violations))
    return run_command(arguments)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
