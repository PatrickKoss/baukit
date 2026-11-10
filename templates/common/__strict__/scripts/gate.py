#!/usr/bin/env python3
"""Run gate steps as parallel lanes, select steps for --affected, and report timings.

The calling script owns the step commands. It lists its steps with
`<script> --list-steps` as lines of `name lane after mode tokens`, where
`after` and `tokens` are comma lists or `-`, a step answers to its name, its
lane and its tokens, and `mode` is `always` (also a
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

from process_runner import run_process

MODES = ("always", "selected", "affected")
DEFAULT_BUDGET_MINUTES = {"full": 10, "affected": 5}
DEFAULT_WORKER_MEMORY_MB = 1024
DEFAULT_ANDROID_EMULATORS = 2
CORES_PER_EMULATOR = 2
LIGHT_LANES = frozenset({"fast"})
EMULATOR_STEPS = frozenset({"android-e2e"})
FAILURE_TAIL_LINES = 40
NOT_RUN = "not run"
STEP_FIELD_COUNT = 5
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
class BudgetLimit:
    minutes: int
    mode: str


@dataclass(frozen=True)
class RunContext:
    command: list[str]
    log_directory: Path
    environment: dict[str, str]


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
        if len(fields) != STEP_FIELD_COUNT:
            raise ValueError(
                f"step line {number} needs name, lane, after, mode and tokens: {line!r}"
            )
        name, lane, after, mode, tokens = fields
        if mode not in MODES:
            raise ValueError(f"step {name} has unknown mode {mode}")
        steps.append(
            Step(name, lane, split_list(after), mode, frozenset({name, lane, *split_list(tokens)}))
        )
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
    return sorted(
        {
            token
            for tokens in affected.values()
            for token in tokens
            if base_token(token) not in known
        }
    )


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


def workspace_members(
    metadata: dict, repository_root: Path
) -> tuple[dict[str, str], dict[str, set[str]]]:
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


def is_light(step: Step, light: frozenset[str]) -> bool:
    return step.lane in LIGHT_LANES or step.lane in light or step.name in light


def heavy_lanes(planned: list[PlannedStep], light: frozenset[str] = frozenset()) -> list[str]:
    lanes = []
    for item in planned:
        if not is_light(item.step, light) and item.step.lane not in lanes:
            lanes.append(item.step.lane)
    return lanes


def unknown_light(light: frozenset[str], steps: list[Step]) -> list[str]:
    known = {step.name for step in steps} | {step.lane for step in steps}
    return sorted(light - known)


class CoreBudget:
    """One pool of cores that every heavy step draws its workers from while it runs."""

    def __init__(
        self, cores: int, memory_mb: int | None, worker_memory_mb: int, emulator_cores: int
    ):
        self.cores = max(1, cores)
        self.free = self.cores
        self.memory_mb = memory_mb
        self.worker_memory_mb = worker_memory_mb
        self.emulator_cores = min(self.cores, max(1, emulator_cores))
        self.condition = threading.Condition()

    def share(self, lanes: int) -> int:
        return split_workers(self.cores, lanes, self.memory_mb, self.worker_memory_mb)

    def acquire(self, want: int, minimum: int = 1) -> int:
        want = min(self.cores, max(1, want))
        minimum = min(want, max(1, minimum))
        with self.condition:
            self.condition.wait_for(lambda: self.free >= minimum)
            granted = min(want, self.free)
            self.free -= granted
            return granted

    def available(self) -> int:
        with self.condition:
            return self.free

    def release(self, cores: int) -> None:
        with self.condition:
            self.free += cores
            self.condition.notify_all()


def budget_report(
    timings: dict[str, int],
    lanes: dict[str, list[str]],
    lane_seconds: dict[str, int],
    wall: int,
    limit: BudgetLimit,
) -> list[str]:
    lines = ["gate: step timings, slowest first"]
    width = max((len(name) for name in timings), default=4)
    for name, seconds in sorted(timings.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"  {name.ljust(width)}  {seconds:>6} s")
    for lane, seconds in sorted(lane_seconds.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"gate: lane {lane} took {seconds} s")
    budget = limit.minutes * 60
    if wall <= budget:
        lines.append(f"gate: {wall} s is within the {limit.minutes} minute {limit.mode} budget")
        return lines
    lines.append(
        f"gate: warning: {wall} s is over the {limit.minutes} minute {limit.mode} budget "
        f"by {wall - budget} s"
    )
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
    result = run_process(["git", *arguments], capture_output=True, text=True, check=False)
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


def light_settings(settings: dict) -> frozenset[str]:
    return frozenset(settings.get("quality", {}).get("light", []))


def emulator_cores(settings: dict, environment: dict[str, str]) -> int:
    configured = environment.get("BAUKIT_GATE_EMULATOR_CORES")
    if configured:
        return int(configured)
    emulators = environment.get("BAUKIT_QA_ANDROID_EMULATORS") or settings.get("quality", {}).get(
        "android_emulators", DEFAULT_ANDROID_EMULATORS
    )
    return CORES_PER_EMULATOR * int(emulators)


def budget_minutes(settings: dict, mode: str) -> int:
    configured = settings.get("quality", {}).get("gate_budget_minutes", {})
    return int(configured.get(mode, DEFAULT_BUDGET_MINUTES[mode]))


class Runner:
    def __init__(
        self,
        planned: list[PlannedStep],
        context: RunContext,
        *,
        budget: CoreBudget | None = None,
        light: frozenset[str] = frozenset(),
    ):
        self.planned = planned
        self.budget = budget or CoreBudget(
            int(context.environment.get("BAUKIT_GATE_WORKERS", "1")), None, 0, 1
        )
        self.light = light
        self.open_lanes = set(heavy_lanes(planned, light))
        self.command = context.command
        self.log_directory = context.log_directory
        self.environment = context.environment
        self.names = {item.step.name for item in planned}
        self.done = {item.step.name: threading.Event() for item in planned}
        self.status: dict[str, str] = {}
        self.timings: dict[str, int] = {}
        self.lane_seconds: dict[str, int] = {}
        self.lock = threading.Lock()
        self.timings_file = context.log_directory / "timings.txt"
        self.started = time.monotonic()

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
                self.status[step.name] = NOT_RUN
                self.done[step.name].set()
                continue
            dependency = self.blocked_by(step)
            if dependency is not None:
                print(
                    f"gate: [{lane}] {step.name} not run because {dependency} did not pass",
                    flush=True,
                )
                self.status[step.name] = f"blocked by {dependency}"
                self.done[step.name].set()
                failed = True
                continue
            failed = not self.run_step(lane, step, log)
        with self.lock:
            self.open_lanes.discard(lane)
        self.lane_seconds[lane] = round(time.monotonic() - started)

    def cores_for(self, step: Step) -> tuple[int, bool]:
        """Return the step's workers and whether they came out of the shared budget."""
        with self.lock:
            share = self.budget.share(len(self.open_lanes))
        if is_light(step, self.light):
            return share, False
        if step.name in EMULATOR_STEPS:
            reserve = self.budget.emulator_cores
            return self.budget.acquire(max(share, reserve), reserve), True
        return self.budget.acquire(share), True

    def grant_detail(self, asked: float) -> str:
        now = time.monotonic()
        return (
            f"at +{round(now - self.started)} s after waiting {round(now - asked)} s, "
            f"{self.budget.available()} of {self.budget.cores} cores free"
        )

    def run_step(self, lane: str, step: Step, log: Path) -> bool:
        asked = time.monotonic()
        cores, drawn = self.cores_for(step)
        environment = dict(self.environment)
        worker_environment(environment, cores)
        print(
            f"gate: [{lane}] {step.name} started with {cores} "
            f"{'cores' if drawn else 'cores, light'}, "
            f"{self.grant_detail(asked)}",
            flush=True,
        )
        started = time.monotonic()
        try:
            with log.open("a") as output:
                output.write(f"\n==> {step.name} ({cores} cores)\n")
                output.flush()
                result = run_process(
                    [*self.command, step.name],
                    stdout=output,
                    stderr=subprocess.STDOUT,
                    env=environment,
                    check=False,
                )
        finally:
            if drawn:
                self.budget.release(cores)
        seconds = round(time.monotonic() - started)
        passed = result.returncode == 0
        self.record(
            step.name, seconds, "passed" if passed else f"failed with exit {result.returncode}"
        )
        if passed:
            print(f"gate: [{lane}] {step.name} passed in {seconds} s", flush=True)
            return True
        print(
            f"gate: [{lane}] {step.name} failed with exit {result.returncode} "
            f"in {seconds} s; log {log}",
            flush=True,
        )
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
            broken = [
                f"{step.name} {self.status.get(step.name, NOT_RUN)}"
                for step in steps
                if self.status.get(step.name) != "passed"
            ]
            if broken:
                failures.append(f"{lane} ({'; '.join(broken)})")
        with self.timings_file.open("a") as output:
            for lane, seconds in self.lane_seconds.items():
                output.write(f"lane:{lane} {seconds}\n")
        return failures


def print_plan(
    mode: str, base: str | None, selection: Selection, planned: list[PlannedStep], extras: list[str]
) -> None:
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
    lines = [
        f"CARGO_HOME {environment['CARGO_HOME']}",
        f"GRADLE_USER_HOME {environment['GRADLE_USER_HOME']}",
    ]
    if "RUSTC_WRAPPER" not in environment and shutil.which("sccache"):
        environment["RUSTC_WRAPPER"] = "sccache"
    wrapper = environment.get("RUSTC_WRAPPER", "")
    lines.append("sccache on" if Path(wrapper).name == "sccache" else "sccache off (not on PATH)")
    return [f"cache: {line}" for line in lines]


def worker_environment(environment: dict[str, str], workers: int) -> None:
    for name in (
        "BAUKIT_GATE_WORKERS",
        "CARGO_BUILD_JOBS",
        "NEXTEST_TEST_THREADS",
        "RUST_TEST_THREADS",
        "VITEST_MAX_WORKERS",
        "E2E_WORKERS",
        "BAUKIT_QA_GRADLE_WORKERS",
        "BAUKIT_QA_NATIVE_JOBS",
    ):
        environment[name] = str(workers)


def rust_packages(
    cargo_workspace: str | None,
    mode: str,
    selection: Selection,
    changed: list[str],
    environment: dict[str, str],
) -> str | None:
    environment["BAUKIT_GATE_RUST_PACKAGES"] = ""
    if not (
        cargo_workspace
        and mode == "affected"
        and "backend" in {base_token(t) for t in selection.tokens}
    ):
        return None
    manifest = Path(cargo_workspace)
    metadata = json.loads(
        run_process(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
                str(manifest),
            ],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    members, dependents = workspace_members(metadata, Path.cwd())
    packages = affected_packages(changed, manifest.parent.as_posix(), members, dependents)
    environment["BAUKIT_GATE_RUST_PACKAGES"] = " ".join(packages or [])
    return "rust packages: " + (", ".join(packages) if packages else "all")


def worker_budget(
    planned: list[PlannedStep],
    light: frozenset[str],
    settings: dict,
    environment: dict[str, str],
    extras: list[str],
) -> CoreBudget:
    cores = int(os.environ.get("BAUKIT_GATE_CORES") or os.cpu_count() or 1)
    lanes = heavy_lanes(planned, light)
    worker_memory = int(os.environ.get("BAUKIT_GATE_WORKER_MEMORY_MB", DEFAULT_WORKER_MEMORY_MB))
    budget = CoreBudget(
        cores, total_memory_mb(), worker_memory, emulator_cores(settings, environment)
    )
    workers = budget.share(len(lanes))
    worker_environment(environment, workers)
    extras.append(
        f"workers: {cores} cores across {len(lanes)} lanes "
        f"({', '.join(lanes) or 'none'}): {workers} per lane"
    )
    extras.append(
        f"workers: lanes share {budget.cores} cores; a finished lane frees its share; "
        f"{', '.join(sorted(EMULATOR_STEPS))} keeps {budget.emulator_cores} for the emulators; "
        f"light: {', '.join(sorted(LIGHT_LANES | light))}"
    )
    return budget


def run_command(arguments: argparse.Namespace) -> int:
    script = shlex.split(arguments.script)
    listing = run_process(
        [*script, "--list-steps"], capture_output=True, text=True, check=True
    ).stdout
    steps = parse_steps(listing)
    settings = load_settings(Path(arguments.settings))
    mode = "affected" if arguments.affected else "full"
    light = light_settings(settings)
    unknown = unknown_light(light, steps)
    if unknown:
        print(
            f"gate: quality.light names unknown lanes or steps: {', '.join(unknown)}",
            file=sys.stderr,
        )
        return 2
    if arguments.affected:
        changed = changed_files(arguments.affected)
        affected = settings.get("quality", {}).get("affected", {})
        unknown = unknown_tokens(affected, steps)
        if unknown:
            print(
                f"gate: quality.affected names unknown lanes or steps: {', '.join(unknown)}",
                file=sys.stderr,
            )
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
    environment["BAUKIT_GATE_STEPS"] = " ".join(item.step.name for item in planned)
    packages = rust_packages(arguments.cargo_workspace, mode, selection, changed, environment)
    extras = [packages] if packages else []
    budget = worker_budget(planned, light, settings, environment, extras)
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
    runner = Runner(
        planned,
        RunContext([*script, "--step"], log_directory, environment),
        budget=budget,
        light=light,
    )
    failures = runner.run()
    wall = round(time.monotonic() - started)
    with runner.timings_file.open("a") as output:
        output.write(f"total {wall}\n")
    lane_steps = {lane: [step.name for step in steps] for lane, steps in runner.lanes().items()}
    report = budget_report(
        runner.timings,
        lane_steps,
        runner.lane_seconds,
        wall,
        BudgetLimit(budget_minutes(settings, mode), mode),
    )
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
