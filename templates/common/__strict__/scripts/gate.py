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

from gate_disk import disk_error
from gate_locks import (
    POOL_POLL_SECONDS,
    CorePool,
    Need,
    ResourceLocks,
    ResourceRule,
    describe_waits,
    resource_map,
    resources_for,
    unknown_resource_members,
)
from gate_results import (
    CachedRun,
    CacheRequest,
    InputMap,
    Machinery,
    StepSpec,
    cached_steps,
    open_cache,
    run_git,
)
from gate_results import settings_error as cache_settings_error
from process_runner import run_process

MODES = ("always", "selected", "affected")
DEFAULT_BUDGET_MINUTES = {"full": 10, "affected": 5}
DEFAULT_WORKER_MEMORY_MB = 1024
DEFAULT_ANDROID_EMULATORS = 2
CORES_PER_EMULATOR = 2
EMULATOR_STEPS = frozenset({"android-e2e"})
DEFAULT_WAITS_FOR = {"android-e2e": ("backend", "fuzz", "images")}
FAILURE_TAIL_LINES = 40
NOT_RUN = "not run"
FAST_LANE = "fast"
FAST_LANE_FAILED = f"{NOT_RUN} (fast lane failed)"
LIGHT_LANES = frozenset({FAST_LANE})
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
    worker_caps: dict[str, int] = field(default_factory=dict)
    cached: CachedRun = field(default_factory=CachedRun)


@dataclass(frozen=True)
class Step:
    name: str
    lane: str
    after: tuple[str, ...]
    mode: str
    tokens: frozenset[str]


@dataclass(frozen=True)
class HostSharing:
    locks: ResourceLocks | None = None
    resources: list[ResourceRule] = field(default_factory=list)
    waits: dict[str, tuple[str, ...]] = field(default_factory=dict)


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


@dataclass(frozen=True)
class EmulatorReserve:
    """Cores held back for emulator steps, and the steps that count as emulator steps."""

    cores: int
    steps: frozenset[str] = EMULATOR_STEPS


class CoreBudget:
    """One pool of cores that every heavy step draws its workers from while it runs."""

    def __init__(
        self,
        cores: int,
        memory_mb: int | None,
        worker_memory_mb: int,
        emulators: EmulatorReserve,
        pool: CorePool | None = None,
    ):
        self.cores = max(1, cores)
        self.pool = pool
        self.free = self.cores
        self.memory_mb = memory_mb
        self.worker_memory_mb = worker_memory_mb
        self.emulator_cores = min(self.cores, max(1, emulators.cores))
        self.emulator_steps = emulators.steps
        self.reserved = 0
        self.minimums: dict[str, int] = {}
        self.held_for_minimums = 0
        self.condition = threading.Condition()

    def reserve(self, cores: int) -> None:
        """Hold cores back from other steps while an emulator step waits for its reserve."""
        with self.condition:
            self.reserved = min(self.cores - 1, max(0, cores))
            self.condition.notify_all()

    def require_minimums(self, minimums: dict[str, int]) -> None:
        """Set each named step's minimum cores, clamped to the budget."""
        self.minimums = {name: min(self.cores, max(1, cores)) for name, cores in minimums.items()}

    def hold_for_minimums(self, cores: int) -> None:
        """Hold cores back from other steps while steps with a minimum wait for it."""
        with self.condition:
            self.held_for_minimums = max(0, cores)
            self.condition.notify_all()

    def held_back(self, minimum_step: bool) -> int:
        held = self.reserved if minimum_step else self.reserved + self.held_for_minimums
        return min(self.cores - 1, held)

    def share(self, lanes: int) -> int:
        cores = self.cores
        if self.pool is not None:
            cores = max(1, cores // self.pool.live_gates())
        return split_workers(cores, lanes, self.memory_mb, self.worker_memory_mb)

    def weighted_share(self, lanes: int, seconds: int, total_seconds: int) -> int:
        even = self.share(lanes)
        if seconds <= 0 or total_seconds <= 0:
            return even
        ceiling = self.share(1)
        weighted = -(-ceiling * seconds // total_seconds)
        return max(even, min(ceiling, weighted, max(1, self.available())))

    def acquire(
        self, want: int, minimum: int = 1, *, emulator: bool = False, minimum_step: bool = False
    ) -> int:
        """Steps with a configured minimum skip each other's holds, so two of them
        never wait on cores held for the other."""
        want = min(self.cores, max(1, want))
        minimum = min(want, max(1, minimum))
        if self.pool is not None:
            return self.acquire_from_pool(want, minimum, emulator or minimum_step)
        with self.condition:

            def usable() -> int:
                return self.free if emulator else self.free - self.held_back(minimum_step)

            self.condition.wait_for(lambda: usable() >= minimum)
            granted = min(want, usable())
            self.free -= granted
            return granted

    def acquire_from_pool(self, want: int, minimum: int, exempt: bool) -> int:
        """Leave the cores held for waiting minimums in the shared pool; the emulator
        reserve stays local to this gate, as before."""
        pool = self.pool
        if pool is None:
            return 0
        while not exempt:
            with self.condition:
                held = self.held_for_minimums
                if held == 0:
                    break
                usable = pool.available() - min(self.cores - 1, held)
                if usable >= minimum:
                    want = min(want, usable)
                    break
                self.condition.wait(POOL_POLL_SECONDS)
        return pool.acquire(want, minimum)

    def available(self) -> int:
        if self.pool is not None:
            return self.pool.available()
        with self.condition:
            return self.free

    def release(self, cores: int) -> None:
        if self.pool is not None:
            self.pool.release(cores)
            return
        with self.condition:
            self.free += cores
            self.condition.notify_all()


def read_expected_seconds(path: Path, names: set[str] | None = None) -> dict[str, int]:
    try:
        lines = path.read_text().splitlines()
    except (OSError, UnicodeDecodeError):
        return {}
    expected: dict[str, int] = {}
    for line in lines:
        name, _, value = line.partition(" ")
        if name and (names is None or name in names) and value.isdecimal() and int(value) > 0:
            expected[name] = int(value)
    return expected


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
    configured = environment.get("BAUKIT_GATE_EMULATOR_CORES") or settings.get("quality", {}).get(
        "emulator_cores"
    )
    if configured:
        return int(configured)
    emulators = environment.get("BAUKIT_QA_ANDROID_EMULATORS") or settings.get("quality", {}).get(
        "android_emulators", DEFAULT_ANDROID_EMULATORS
    )
    return CORES_PER_EMULATOR * int(emulators)


def min_cores(settings: dict) -> dict[str, int]:
    return dict(settings.get("quality", {}).get("min_cores", {}))


def min_cores_error(settings: dict, steps: list[Step]) -> str | None:
    configured = settings.get("quality", {}).get("min_cores", {})
    if not isinstance(configured, dict):
        return "gate: quality.min_cores must be a table of step names to positive integers"
    names = {step.name for step in steps}
    for name, cores in configured.items():
        if name not in names:
            return f"gate: quality.min_cores names unknown step {name}"
        if isinstance(cores, bool) or not isinstance(cores, int) or cores < 1:
            return f"gate: quality.min_cores.{name} must be a positive integer, got {cores!r}"
    return None


def emulator_steps(settings: dict, environment: dict[str, str]) -> frozenset[str]:
    if environment.get("BAUKIT_QA_DEVICE_BROKER"):
        return frozenset()
    return EMULATOR_STEPS | frozenset(settings.get("quality", {}).get("emulator_steps", []))


def waits_for_settings(settings: dict) -> dict[str, tuple[str, ...]]:
    configured = settings.get("quality", {}).get("waits_for", {})
    return {**DEFAULT_WAITS_FOR, **{name: tuple(members) for name, members in configured.items()}}


def step_waits(
    steps: list[Step], waits_for: dict[str, tuple[str, ...]]
) -> dict[str, tuple[str, ...]]:
    """Map each step to the steps of other lanes it starts after, whether they pass or not."""
    waits = {}
    for step in steps:
        members = set(waits_for.get(step.name, ()))
        waited = tuple(
            other.name
            for other in steps
            if other.lane != step.lane and {other.name, other.lane} & members
        )
        if waited:
            waits[step.name] = waited
    return waits


def predecessors(steps: list[Step], waits: dict[str, tuple[str, ...]]) -> dict[str, list[str]]:
    """Map each step to the steps that must end before it starts: lane order, after and waits."""
    before: dict[str, list[str]] = {}
    last_in_lane: dict[str, str] = {}
    for step in steps:
        before[step.name] = [*step.after, *waits.get(step.name, ())]
        if step.lane in last_in_lane:
            before[step.name].append(last_in_lane[step.lane])
        last_in_lane[step.lane] = step.name
    return before


def runs_after(start: str, target: str, before: dict[str, list[str]]) -> bool:
    seen: set[str] = set()
    pending = [start]
    while pending:
        name = pending.pop()
        if name == target:
            return True
        if name not in seen:
            seen.add(name)
            pending.extend(before.get(name, ()))
    return False


def waits_for_error(steps: list[Step], settings: dict) -> str | None:
    configured = settings.get("quality", {}).get("waits_for", {})
    by_name = {step.name: step for step in steps}
    lanes = {step.lane for step in steps}
    for name, members in configured.items():
        if by_name.get(name) is None or by_name[name].lane == FAST_LANE:
            return f"gate: quality.waits_for names {name}, which is no step outside the fast lane"
        unknown = ", ".join(sorted(set(members) - by_name.keys() - lanes))
        if unknown:
            return f"gate: quality.waits_for.{name} names unknown lanes or steps: {unknown}"
    waits = step_waits(steps, waits_for_settings(settings))
    before = predecessors(steps, waits)
    for name, waited in waits.items():
        cycle = next((other for other in waited if runs_after(other, name, before)), None)
        if cycle:
            return f"gate: {name} waits for {cycle}, which runs after {name}"
    return None


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
        sharing: HostSharing | None = None,
    ):
        self.planned = planned
        self.result_cache = context.cached.cache
        self.cached = context.cached.passes
        running = [item for item in planned if item.step.name not in self.cached]
        sharing = sharing or HostSharing()
        self.locks = sharing.locks
        self.resources = sharing.resources
        self.waits = sharing.waits
        self.fast_done = threading.Event()
        self.fast_passed = True
        if FAST_LANE not in {item.step.lane for item in planned}:
            self.fast_done.set()
        self.budget = budget or CoreBudget(
            int(context.environment.get("BAUKIT_GATE_WORKERS", "1")), None, 0, EmulatorReserve(1)
        )
        self.light = light
        self.open_lanes = set(heavy_lanes(running, light))
        self.command = context.command
        self.log_directory = context.log_directory
        self.environment = context.environment
        self.worker_caps = context.worker_caps
        self.names = {item.step.name for item in planned}
        self.done = {item.step.name: threading.Event() for item in planned}
        self.status: dict[str, str] = {}
        self.timings: dict[str, int] = {}
        self.lane_seconds: dict[str, int] = {}
        self.lock = threading.Lock()
        self.timings_file = context.log_directory / "timings.txt"
        self.history_file = context.log_directory / "step-seconds.txt"
        self.history = read_expected_seconds(self.history_file)
        self.expected = {n: v for n, v in self.history.items() if n in self.names}
        self.pending = {item.step.name for item in running if not is_light(item.step, light)}
        self.emulators_waiting = {
            item.step.name for item in running if item.step.name in self.budget.emulator_steps
        }
        self.emulators_running = 0
        self.minimums_waiting = {
            item.step.name
            for item in running
            if item.step.name in self.budget.minimums and not is_light(item.step, light)
        }
        self.started = time.monotonic()
        self.update_reserve()

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
            if status == "passed":
                self.save_history(name, seconds)
            self.emulators_waiting.discard(name)
            self.minimums_waiting.discard(name)
            self.update_reserve()
        self.done[name].set()

    def step_ready(self, name: str) -> bool:
        step = next(item.step for item in self.planned if item.step.name == name)
        return (
            name not in self.status
            and all(
                self.status.get(dependency) == "passed"
                for dependency in step.after
                if dependency in self.names
            )
            and all(waited in self.status for waited in self.waits.get(name, ()))
        )

    def update_reserve(self) -> None:
        """Reserve emulator cores once an emulator step's dependencies passed, before
        the steps that waited on the same dependencies wake up and drain the pool."""
        ready = any(self.step_ready(name) for name in self.emulators_waiting)
        waiting = ready and self.emulators_running == 0
        self.budget.reserve(self.budget.emulator_cores if waiting else 0)
        self.budget.hold_for_minimums(
            sum(
                self.budget.minimums[name]
                for name in self.minimums_waiting
                if self.step_ready(name)
            )
        )

    def save_history(self, name: str, seconds: int) -> None:
        self.history[name] = max(1, seconds)
        scratch = self.history_file.with_name(self.history_file.name + ".tmp")
        scratch.write_text("".join(f"{key} {value}\n" for key, value in self.history.items()))
        os.replace(scratch, self.history_file)

    def blocked_by(self, step: Step) -> str | None:
        for dependency in step.after:
            if dependency not in self.names:
                continue
            self.done[dependency].wait()
            if self.status[dependency] != "passed":
                return dependency
        return None

    def wait_for_quiet(self, lane: str, step: Step) -> None:
        busy = [name for name in self.waits.get(step.name, ()) if not self.done[name].is_set()]
        if not busy:
            return
        print(f"gate: [{lane}] {step.name} waits for {', '.join(busy)} to finish", flush=True)
        started = time.monotonic()
        for name in busy:
            self.done[name].wait()
        print(
            f"gate: [{lane}] {step.name} stopped waiting after "
            f"{round(time.monotonic() - started)} s",
            flush=True,
        )

    def step_resources(self, step: Step) -> list[Need]:
        if self.locks is None:
            return []
        return resources_for({step.name, step.lane}, self.resources)

    def skip_after_fast_failure(self, lane: str, step: Step) -> bool:
        if lane == FAST_LANE or not self.step_resources(step):
            return False
        self.fast_done.wait()
        if self.fast_passed:
            return False
        print(f"gate: [{lane}] {step.name} {FAST_LANE_FAILED}", flush=True)
        self.status[step.name] = FAST_LANE_FAILED
        self.done[step.name].set()
        return True

    def run_lane(self, lane: str, steps: list[Step]) -> None:
        log = self.log_directory / f"{lane}.log"
        started = time.monotonic()
        failed = False
        try:
            failed = self.run_steps(lane, steps, log)
        finally:
            if lane == FAST_LANE:
                self.fast_passed = not failed
                self.fast_done.set()
        with self.lock:
            self.open_lanes.discard(lane)
        self.lane_seconds[lane] = round(time.monotonic() - started)

    def pass_from_cache(self, lane: str, step: Step) -> None:
        detail = self.cached[step.name].describe(time.time())
        print(f"gate: [{lane}] {step.name} cached ({detail})", flush=True)
        with self.lock:
            self.status[step.name] = "passed"
        self.done[step.name].set()

    def remember_pass(self, step: Step, seconds: int, log: Path) -> None:
        if self.result_cache is not None and self.result_cache.store(step.name, seconds, log):
            print(f"gate: {step.name} pass stored in the result cache", flush=True)

    def run_steps(self, lane: str, steps: list[Step], log: Path) -> bool:
        failed = False
        for step in steps:
            if step.name in self.cached:
                self.pass_from_cache(lane, step)
                continue
            if failed:
                self.status[step.name] = NOT_RUN
                self.done[step.name].set()
                continue
            if self.skip_after_fast_failure(lane, step):
                failed = True
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
            self.wait_for_quiet(lane, step)
            failed = not self.run_step(lane, step, log)
        return failed

    def step_share(self, step: Step) -> int:
        lanes = len(self.open_lanes)
        seconds = self.expected.get(step.name, 0)
        if seconds == 0 or step.name not in self.pending:
            self.pending.discard(step.name)
            return self.budget.share(lanes)
        mean = sum(self.expected.values()) // len(self.expected)
        total = sum(self.expected.get(name, mean) for name in self.pending)
        self.pending.discard(step.name)
        return self.budget.weighted_share(lanes, seconds, total)

    def cores_for(self, step: Step) -> tuple[int, bool]:
        """Return the step's workers and whether they came out of the shared budget."""
        with self.lock:
            share = self.step_share(step)
        if is_light(step, self.light):
            return share, False
        if step.name in self.budget.emulator_steps:
            reserve = self.budget.emulator_cores
            cores = self.budget.acquire(max(share, reserve), reserve, emulator=True)
            with self.lock:
                self.emulators_waiting.discard(step.name)
                self.emulators_running += 1
                self.update_reserve()
            return cores, True
        minimum = self.budget.minimums.get(step.name)
        if minimum is not None:
            return self.minimum_cores(step, share, minimum), True
        return self.budget.acquire(share), True

    def minimum_cores(self, step: Step, share: int, minimum: int) -> int:
        cores = self.budget.acquire(max(share, minimum), minimum, minimum_step=True)
        with self.lock:
            self.minimums_waiting.discard(step.name)
            self.update_reserve()
        return cores

    def release_cores(self, step: Step, cores: int) -> None:
        self.budget.release(cores)
        if step.name not in self.budget.emulator_steps:
            return
        with self.lock:
            self.emulators_running -= 1
            self.update_reserve()

    def grant_detail(self, asked: float) -> str:
        now = time.monotonic()
        kind = "pool cores" if self.budget.pool is not None else "cores"
        return (
            f"at +{round(now - self.started)} s after waiting {round(now - asked)} s, "
            f"{self.budget.available()} of {self.budget.cores} {kind} free"
        )

    def run_step(self, lane: str, step: Step, log: Path) -> bool:
        resources = self.step_resources(step)
        if self.locks is None or not resources:
            return self.run_held_step(lane, step, log, [])
        with self.locks.hold(resources) as waits:
            return self.run_held_step(lane, step, log, waits)

    def run_held_step(self, lane: str, step: Step, log: Path, waits: list) -> bool:
        asked = time.monotonic()
        cores, drawn = self.cores_for(step)
        environment = dict(self.environment)
        worker_environment(environment, cores, self.worker_caps)
        print(
            f"gate: [{lane}] {step.name} started with {cores} "
            f"{'cores' if drawn else 'cores, light'}, "
            f"{describe_waits(waits) + ', ' if waits else ''}"
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
                self.release_cores(step, cores)
        seconds = round(time.monotonic() - started)
        passed = result.returncode == 0
        self.record(
            step.name, seconds, "passed" if passed else f"failed with exit {result.returncode}"
        )
        if passed:
            print(f"gate: [{lane}] {step.name} passed in {seconds} s", flush=True)
            self.remember_pass(step, seconds, log)
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


WORKER_VARIABLES = (
    "BAUKIT_GATE_WORKERS",
    "CARGO_BUILD_JOBS",
    "NEXTEST_TEST_THREADS",
    "RUST_TEST_THREADS",
    "VITEST_MAX_WORKERS",
    "E2E_WORKERS",
    "BAUKIT_QA_GRADLE_WORKERS",
    "BAUKIT_QA_NATIVE_JOBS",
)


def worker_caps(settings: dict) -> dict[str, int]:
    gate = settings.get("gate", {})
    caps = gate.get("worker-caps", {}) if isinstance(gate, dict) else {}
    return dict(caps) if isinstance(caps, dict) else {}


def worker_caps_error(settings: dict) -> str | None:
    gate = settings.get("gate", {})
    if not isinstance(gate, dict) or not isinstance(gate.get("worker-caps", {}), dict):
        return "gate: gate.worker-caps must be a table of variable names to positive integers"
    for name, cap in worker_caps(settings).items():
        if name not in WORKER_VARIABLES:
            return (
                f"gate: gate.worker-caps names unknown variable {name}; "
                f"known: {', '.join(WORKER_VARIABLES)}"
            )
        if isinstance(cap, bool) or not isinstance(cap, int) or cap < 1:
            return f"gate: gate.worker-caps.{name} must be a positive integer, got {cap!r}"
    return None


def worker_environment(
    environment: dict[str, str], workers: int, caps: dict[str, int] | None = None
) -> None:
    caps = caps or {}
    for name in WORKER_VARIABLES:
        environment[name] = str(max(1, min(workers, caps.get(name, workers))))


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
    pool = CorePool.from_environment(os.environ, max(1, cores))
    lanes = heavy_lanes(planned, light)
    worker_memory = int(os.environ.get("BAUKIT_GATE_WORKER_MEMORY_MB", DEFAULT_WORKER_MEMORY_MB))
    budget = CoreBudget(
        cores,
        total_memory_mb(),
        worker_memory,
        EmulatorReserve(
            emulator_cores(settings, environment), emulator_steps(settings, environment)
        ),
        pool,
    )
    budget.require_minimums(min_cores(settings))
    if pool is not None:
        extras.append(f"cores: {budget.cores} slots shared through {pool.directory}")
    workers = budget.share(len(lanes))
    worker_environment(environment, workers, worker_caps(settings))
    extras.append(
        f"workers: {cores} cores across {len(lanes)} lanes "
        f"({', '.join(lanes) or 'none'}): {workers} per lane"
    )
    extras.append(
        f"workers: lanes share {budget.cores} cores; a finished lane frees its share; "
        f"{', '.join(sorted(budget.emulator_steps))} keeps {budget.emulator_cores} "
        "for the emulators; "
        f"light: {', '.join(sorted(LIGHT_LANES | light))}"
    )
    if budget.minimums:
        minimums = ", ".join(f"{name} {cores}" for name, cores in sorted(budget.minimums.items()))
        extras.append(f"workers: minimum cores, held back until the step starts: {minimums}")
    return budget


def unknown_names_error(settings: dict, light: frozenset[str], steps: list[Step]) -> str | None:
    unknown = unknown_light(light, steps)
    if unknown:
        return f"gate: quality.light names unknown lanes or steps: {', '.join(unknown)}"
    names = {*(step.name for step in steps), *(step.lane for step in steps)}
    unknown = unknown_resource_members(settings, names)
    if unknown:
        return (
            "gate: quality.resources, quality.shared_resources and quality.creator_resources "
            f"name unknown lanes or steps: {', '.join(unknown)}"
        )
    return None


def fast_lane_wait_error(steps: list[Step], resources: list[ResourceRule]) -> str | None:
    by_name = {step.name: step for step in steps}
    for step in steps:
        if step.lane != FAST_LANE:
            continue
        waiting = first_locking_dependency(step, by_name, resources)
        if waiting:
            return (
                f"gate: fast-lane step {step.name} runs after {waiting}, which takes locks and so "
                f"waits for the fast lane; move {step.name} out of the fast lane"
            )
    return None


def first_locking_dependency(
    step: Step, by_name: dict[str, Step], resources: list[ResourceRule]
) -> str | None:
    seen: set[str] = set()
    pending = list(step.after)
    while pending:
        name = pending.pop()
        dependency = by_name.get(name)
        if dependency is None or name in seen:
            continue
        seen.add(name)
        if dependency.lane != FAST_LANE and resources_for({name, dependency.lane}, resources):
            return name
        pending.extend(dependency.after)
    return None


def prepare_log_directory(path: Path) -> Path:
    directory = path.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    for stale in directory.glob("*.log"):
        stale.unlink()
    return directory


def lock_lines(planned: list[PlannedStep], sharing: HostSharing) -> list[str]:
    if sharing.locks is None:
        return []
    lines = [f"locks: {sharing.locks.directory}; heavy steps wait for the fast lane"]
    for item in planned:
        held = resources_for({item.step.name, item.step.lane}, sharing.resources)
        if held:
            lines.append(
                f"locks: {item.step.name} takes "
                f"{', '.join(need.token.replace('~', ' ') for need in held)}"
            )
    return lines


def wait_lines(waits: dict[str, tuple[str, ...]]) -> list[str]:
    return [f"waits: {name} starts after {', '.join(waited)}" for name, waited in waits.items()]


def host_sharing(
    planned: list[PlannedStep], settings: dict, environment: dict[str, str], extras: list[str]
) -> HostSharing:
    sharing = HostSharing(
        ResourceLocks.from_environment(environment),
        resource_map(settings),
        step_waits([item.step for item in planned], waits_for_settings(settings)),
    )
    extras.extend(lock_lines(planned, sharing))
    extras.extend(wait_lines(sharing.waits))
    return sharing


def gate_machinery(arguments: argparse.Namespace) -> Machinery:
    files = [Path(part) for part in shlex.split(arguments.script) if Path(part).is_file()]
    files.extend(Path(__file__).resolve().parent.glob("*.py"))
    runner_module = sys.modules[run_process.__module__].__file__
    if runner_module:
        files.append(Path(runner_module))
    return Machinery(Path(arguments.settings), tuple(files))


def result_cache(
    arguments: argparse.Namespace,
    planned: list[PlannedStep],
    settings: dict,
    environment: dict[str, str],
    extras: list[str],
) -> tuple[CachedRun, list[PlannedStep]]:
    """Look up cached passes and return them with the steps that really run."""
    request = CacheRequest(
        [StepSpec(item.step.name, item.step.mode, item.step.tokens) for item in planned],
        settings,
        InputMap(
            settings.get("quality", {}).get("affected", {}),
            shared_paths(settings),
            glob_pattern,
            base_token,
        ),
        gate_machinery(arguments),
        environment,
        arguments.no_cache,
    )
    cache, line = open_cache(request, run_git)
    extras.append(line)
    passes = cached_steps(cache, {item.step.name: item.step.after for item in planned})
    running = [item for item in planned if item.step.name not in passes]
    environment["BAUKIT_GATE_STEPS"] = " ".join(item.step.name for item in running)
    return CachedRun(cache, passes), running


def mark_cached(planned: list[PlannedStep], cached: CachedRun) -> list[PlannedStep]:
    now = time.time()
    return [
        PlannedStep(
            item.step, f"{item.reason}; cached, {cached.passes[item.step.name].describe(now)}"
        )
        if item.step.name in cached.passes
        else item
        for item in planned
    ]


def print_cache_notes(cached: CachedRun) -> None:
    for note in cached.cache.notes if cached.cache else []:
        print(f"gate: {note}")


def plan_or_disk_exit(
    plan_only: bool, settings: dict, planned: list[PlannedStep], environment: dict[str, str]
) -> int | None:
    if plan_only:
        return 0
    shortage = disk_error(settings, [item.step.name for item in planned], Path.cwd(), environment)
    if shortage:
        print(shortage, file=sys.stderr)
        return 2
    return None


def run_command(arguments: argparse.Namespace) -> int:
    script = shlex.split(arguments.script)
    listing = run_process(
        [*script, "--list-steps"], capture_output=True, text=True, check=True
    ).stdout
    steps = parse_steps(listing)
    settings = load_settings(Path(arguments.settings))
    mode = "affected" if arguments.affected else "full"
    light = light_settings(settings)
    name_error = (
        unknown_names_error(settings, light, steps)
        or fast_lane_wait_error(steps, resource_map(settings))
        or waits_for_error(steps, settings)
        or worker_caps_error(settings)
        or min_cores_error(settings, steps)
        or cache_settings_error(settings, [step.name for step in steps])
    )
    if name_error:
        print(name_error, file=sys.stderr)
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
    packages = rust_packages(arguments.cargo_workspace, mode, selection, changed, environment)
    extras = [packages] if packages else []
    cached, running = result_cache(arguments, planned, settings, environment, extras)
    budget = worker_budget(running, light, settings, environment, extras)
    extras.extend(cache_environment(environment))
    sharing = host_sharing(running, settings, environment, extras)
    print_plan(mode, arguments.affected, selection, mark_cached(planned, cached), extras)
    if (stop := plan_or_disk_exit(arguments.plan, settings, running, environment)) is not None:
        return stop
    log_directory = prepare_log_directory(Path(arguments.log_dir))
    environment["BAUKIT_GATE_LOG_DIR"] = str(log_directory)
    started = time.monotonic()
    runner = Runner(
        planned,
        RunContext([*script, "--step"], log_directory, environment, worker_caps(settings), cached),
        budget=budget,
        light=light,
        sharing=sharing,
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
    print_cache_notes(cached)
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
    run.add_argument("--no-cache", action="store_true")
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
