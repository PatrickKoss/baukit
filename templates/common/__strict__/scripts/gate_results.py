"""Remember passed gate steps by their inputs, so a re-run can skip them.

A step's key hashes its name, a format version, the gate machinery (settings file,
step script, this directory's Python modules), the run context (`BAUKIT_GATE_MODE`,
the resolved base, `BAUKIT_GATE_TOKENS`, `BAUKIT_GATE_RUST_PACKAGES` and every other
`BAUKIT_*` variable outside `CONTEXT_EXCLUDED`) and the sorted (path, blob id) pairs of
its inputs. The inputs are every tracked file that `--affected` would send to the
step: files whose `[quality.affected]` globs name one of its tokens, files that match
a shared glob, and files that match no affected glob, which `--affected` turns into a
full gate. Steps of mode `always` or `affected` take the whole tracked tree.

Only passes are stored, one JSON file per entry under
`$XDG_CACHE_HOME/baukit/gate-results/<repo id>/`, and only from a clean tree whose
HEAD did not move during the run. `[quality.cache] never` lists steps that read data
from outside the tree, and `max_age_days` (default 7) bounds drift in host tools.
The cache is off with `--no-cache`, when `CI` is set, or with `BAUKIT_GATE_CACHE=off`.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import tempfile
import time
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path

from process_runner import run_process

FORMAT_VERSION = "1"
DEFAULT_MAX_AGE_DAYS = 7
SECONDS_PER_MINUTE = 60
SECONDS_PER_HOUR = 3600
SECONDS_PER_DAY = 86400
SHORT_SHA = 7
REPO_ID_LENGTH = 16
CACHE_SWITCH = "BAUKIT_GATE_CACHE"
EVERY_CHANGE_MODES = frozenset({"always", "affected"})
CONTEXT_PREFIX = "BAUKIT_"
BASE_VARIABLE = "BAUKIT_GATE_BASE"
CONTEXT_EXCLUDED = frozenset(
    {
        CACHE_SWITCH,
        "BAUKIT_GATE_LOG_DIR",
        "BAUKIT_GATE_STEPS",
        "BAUKIT_GATE_CORES",
        "BAUKIT_GATE_WORKERS",
        "BAUKIT_GATE_WORKER_MEMORY_MB",
        "BAUKIT_GATE_DISK_FLOOR_GB",
        "BAUKIT_GATE_LOCK_DIR",
        "BAUKIT_GATE_CORE_POOL",
        "BAUKIT_GATE_EMULATOR_CORES",
        "BAUKIT_HEAVY_SLOT_DIR",
        "BAUKIT_QA_GRADLE_WORKERS",
        "BAUKIT_QA_NATIVE_JOBS",
    }
)
UNSAFE_NAME = re.compile(r"[^A-Za-z0-9_.-]")

Git = Callable[..., "str | None"]
Clock = Callable[[], float]


@dataclass(frozen=True)
class Entry:
    step: str
    key: str
    commit: str
    finished: float
    seconds: int
    log: str

    def describe(self, now: float) -> str:
        return f"passed at {self.commit[:SHORT_SHA]}, {age_text(now - self.finished)}"


@dataclass(frozen=True)
class StepSpec:
    name: str
    mode: str
    tokens: frozenset[str]


@dataclass(frozen=True)
class InputMap:
    """The `--affected` selection map, with gate.py's glob compiler and token parser."""

    affected: Mapping[str, list[str]]
    shared: list[str]
    compile_glob: Callable[[str], re.Pattern[str]]
    base_token: Callable[[str], str]


@dataclass(frozen=True)
class Machinery:
    settings: Path
    files: tuple[Path, ...]


@dataclass
class ResultCache:
    directory: Path
    keys: dict[str, str]
    head: str
    max_age_seconds: int
    git: Git
    clock: Clock = time.time
    notes: list[str] = field(default_factory=list)

    def entry_path(self, name: str) -> Path:
        return self.directory / f"{UNSAFE_NAME.sub('_', name)}-{self.keys[name]}.json"

    def lookup(self, name: str) -> Entry | None:
        if name not in self.keys:
            return None
        entry = read_entry(self.entry_path(name))
        if entry is None or entry.key != self.keys[name] or entry.step != name:
            return None
        if self.clock() - entry.finished > self.max_age_seconds:
            return None
        return entry

    def store(self, name: str, seconds: int, log: Path) -> bool:
        if name not in self.keys:
            return False
        if tree_changed(self.git, self.head):
            self.notes.append(f"cache: {name} not stored, the tree changed during the run")
            return False
        entry = Entry(name, self.keys[name], self.head, self.clock(), seconds, str(log))
        write_entry(self.entry_path(name), entry)
        return True


def run_git(*arguments: str) -> str | None:
    result = run_process(["git", *arguments], capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def age_text(seconds: float) -> str:
    seconds = max(0, round(seconds))
    if seconds < SECONDS_PER_MINUTE:
        return f"{seconds} s ago"
    if seconds < SECONDS_PER_HOUR:
        return f"{seconds // SECONDS_PER_MINUTE} min ago"
    if seconds < SECONDS_PER_DAY:
        return f"{seconds // SECONDS_PER_HOUR} h ago"
    return f"{seconds // SECONDS_PER_DAY} d ago"


def read_entry(path: Path) -> Entry | None:
    try:
        data = json.loads(path.read_text())
        return Entry(
            str(data["step"]),
            str(data["key"]),
            str(data["commit"]),
            float(data["finished"]),
            int(data["seconds"]),
            str(data["log"]),
        )
    except (OSError, ValueError, KeyError, TypeError):
        return None


def write_entry(path: Path, entry: Entry) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "w", dir=path.parent, prefix=f".{path.name}.", suffix=".tmp", delete=False
    ) as scratch:
        json.dump(entry.__dict__, scratch)
    os.replace(scratch.name, path)


def cache_settings(settings: dict) -> dict:
    return settings.get("quality", {}).get("cache", {})


def never_steps(settings: dict) -> frozenset[str]:
    return frozenset(cache_settings(settings).get("never", []))


def max_age_seconds(settings: dict) -> int:
    days = cache_settings(settings).get("max_age_days", DEFAULT_MAX_AGE_DAYS)
    return int(days) * SECONDS_PER_DAY


def settings_error(settings: dict, step_names: Iterable[str]) -> str | None:
    unknown = sorted(never_steps(settings) - set(step_names))
    if unknown:
        return f"gate: quality.cache.never names unknown steps: {', '.join(unknown)}"
    days = cache_settings(settings).get("max_age_days", DEFAULT_MAX_AGE_DAYS)
    if isinstance(days, bool) or not isinstance(days, int) or days < 0:
        return f"gate: quality.cache.max_age_days must be a whole number of days, got {days!r}"
    return None


def disabled_reason(environment: Mapping[str, str], no_cache: bool) -> str | None:
    if no_cache:
        return "--no-cache"
    if environment.get("CI"):
        return "CI is set"
    if environment.get(CACHE_SWITCH, "").lower() == "off":
        return f"{CACHE_SWITCH}=off"
    return None


def path_dependency_reason(settings: dict) -> str | None:
    baukit = settings.get("dependencies", {}).get("baukit", {})
    if isinstance(baukit, dict) and baukit.get("source") == "path":
        return "dependencies.baukit is a path source outside this repository"
    return None


def dirty_reason(git: Git) -> str | None:
    status = git("status", "--porcelain", "--untracked-files=all")
    if status is None:
        return "git status failed"
    return "the tree has uncommitted or untracked files" if status.strip() else None


def tree_changed(git: Git, head: str) -> bool:
    return dirty_reason(git) is not None or git("rev-parse", "HEAD") != head


def tracked_blobs(git: Git) -> dict[str, str]:
    listing = git("ls-files", "-s", "-z", "--full-name", "--", ":/") or ""
    blobs = {}
    for record in listing.split("\0"):
        meta, _, path = record.partition("\t")
        fields = meta.split()
        if path and len(fields) > 1:
            blobs[path] = fields[1]
    return blobs


def classify(paths: Iterable[str], inputs: InputMap) -> dict[str, frozenset[str] | None]:
    """Map each path to the base tokens `--affected` gives it, or None for a full gate."""
    shared = [inputs.compile_glob(glob) for glob in inputs.shared]
    affected = [(inputs.compile_glob(glob), tokens) for glob, tokens in inputs.affected.items()]
    classes: dict[str, frozenset[str] | None] = {}
    for path in paths:
        if any(pattern.fullmatch(path) for pattern in shared):
            classes[path] = None
            continue
        hits = [tokens for pattern, tokens in affected if pattern.fullmatch(path)]
        if not hits:
            classes[path] = None
            continue
        classes[path] = frozenset(inputs.base_token(token) for tokens in hits for token in tokens)
    return classes


def step_inputs(step: StepSpec, classes: Mapping[str, frozenset[str] | None]) -> list[str]:
    if step.mode in EVERY_CHANGE_MODES:
        return sorted(classes)
    return sorted(
        path for path, tokens in classes.items() if tokens is None or tokens & step.tokens
    )


def sha256(parts: Iterable[str]) -> str:
    digest = hashlib.sha256()
    for part in parts:
        digest.update(part.encode())
        digest.update(b"\0")
    return digest.hexdigest()


def machinery_digest(machinery: Machinery) -> str:
    digest = hashlib.sha256()
    for path in sorted({machinery.settings, *machinery.files}):
        digest.update(path.name.encode() + b"\0")
        try:
            digest.update(path.read_bytes())
        except OSError:
            digest.update(b"missing")
        digest.update(b"\0")
    return digest.hexdigest()


def run_context(environment: Mapping[str, str], git: Git) -> list[str]:
    context = []
    for name in sorted(environment):
        if not name.startswith(CONTEXT_PREFIX) or name in CONTEXT_EXCLUDED:
            continue
        value = environment[name]
        if name == BASE_VARIABLE and value:
            value = git("merge-base", value, "HEAD") or git("rev-parse", value) or value
        context.append(f"{name}={value}")
    return context


def step_key(step: StepSpec, shared_parts: list[str], inputs: list[str], blobs: dict) -> str:
    pairs = (f"{path}\t{blobs[path]}" for path in inputs)
    return sha256([FORMAT_VERSION, step.name, *shared_parts, *pairs])


def repo_id(git: Git) -> str:
    origin = git("config", "--get", "remote.origin.url")
    source = origin or git("rev-parse", "--path-format=absolute", "--git-common-dir") or ""
    return hashlib.sha256(source.encode()).hexdigest()[:REPO_ID_LENGTH]


def cache_root(environment: Mapping[str, str]) -> Path:
    base = environment.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "baukit" / "gate-results"


@dataclass(frozen=True)
class CacheRequest:
    steps: list[StepSpec]
    settings: dict
    inputs: InputMap
    machinery: Machinery
    environment: Mapping[str, str]
    no_cache: bool = False


def off_reason(request: CacheRequest, git: Git) -> str | None:
    return (
        disabled_reason(request.environment, request.no_cache)
        or path_dependency_reason(request.settings)
        or dirty_reason(git)
    )


def open_cache(
    request: CacheRequest, git: Git, clock: Clock = time.time
) -> tuple[ResultCache | None, str]:
    reason = off_reason(request, git)
    head = git("rev-parse", "HEAD")
    if reason is None and head is None:
        reason = "HEAD does not resolve"
    if reason is not None or head is None:
        return None, f"cache: off ({reason}); every step runs"
    blobs = tracked_blobs(git)
    classes = classify(blobs, request.inputs)
    shared_parts = [
        machinery_digest(request.machinery),
        *run_context(request.environment, git),
    ]
    never = never_steps(request.settings)
    keys = {
        step.name: step_key(step, shared_parts, step_inputs(step, classes), blobs)
        for step in request.steps
        if step.name not in never
    }
    directory = cache_root(request.environment) / repo_id(git)
    cache = ResultCache(directory, keys, head, max_age_seconds(request.settings), git, clock)
    never_text = f"; never cached: {', '.join(sorted(never))}" if never else ""
    return cache, f"cache: on in {directory}{never_text}"


def cached_steps(
    cache: ResultCache | None, after: Mapping[str, tuple[str, ...]]
) -> dict[str, Entry]:
    """Return the cached passes, minus any step a step that really runs depends on.

    A cached step produces nothing, so a step that runs after it must find what the
    provider would have built (installed packages, a build) and the provider runs too.
    """
    if cache is None:
        return {}
    cached = {name: entry for name in after if (entry := cache.lookup(name)) is not None}
    changed = True
    while changed:
        changed = False
        for name, dependencies in after.items():
            if name in cached:
                continue
            for dependency in dependencies:
                if dependency in cached:
                    del cached[dependency]
                    changed = True
    return cached


@dataclass(frozen=True)
class CachedRun:
    cache: ResultCache | None = None
    passes: dict[str, Entry] = field(default_factory=dict)
