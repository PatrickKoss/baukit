"""Refuse to start the gate when the worktree or Docker filesystem is short on space.

`[quality.disk]` sets `floor_gb`, the free space every gate run needs, and
`step_floor_gb`, a table of higher floors for steps that start k3d or many
containers. A run needs the highest floor among its planned steps, and
`BAUKIT_GATE_DISK_FLOOR_GB` overrides it for one run (0 turns the check off). The check
only prints hints and never deletes anything.
"""

from __future__ import annotations

import shutil
import subprocess
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path

from process_runner import run_process

DEFAULT_FLOOR_GB = 40
BYTES_PER_GB = 1024**3
TARGET_HINTS = 5
TARGET_SEARCH_DEPTH = 4
COMMAND_TIMEOUT_SECONDS = 30

Command = Callable[[list[str]], str | None]
FreeBytes = Callable[[Path], int]


@dataclass(frozen=True)
class Mount:
    path: Path
    free_bytes: int


def go_template(name: str) -> str:
    # The template renderer would read two literal braces as an expression.
    return "{" * 2 + name + "}" * 2


def run_quiet(arguments: list[str]) -> str | None:
    if shutil.which(arguments[0]) is None:
        return None
    try:
        result = run_process(
            arguments,
            capture_output=True,
            text=True,
            check=False,
            timeout=COMMAND_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def free_bytes(path: Path) -> int:
    return shutil.disk_usage(path).free


@dataclass(frozen=True)
class Probe:
    command: Command = run_quiet
    measure: FreeBytes = free_bytes


def required_floor_gb(
    settings: dict, step_names: Iterable[str], environment: dict[str, str]
) -> int:
    override = environment.get("BAUKIT_GATE_DISK_FLOOR_GB")
    if override:
        return int(override)
    disk = settings.get("quality", {}).get("disk", {})
    step_floors = disk.get("step_floor_gb", {})
    floors = [int(disk.get("floor_gb", DEFAULT_FLOOR_GB))]
    floors.extend(int(step_floors[name]) for name in step_names if name in step_floors)
    return max(floors)


def docker_root(command: Command) -> Path | None:
    output = command(["docker", "info", "--format", go_template(".DockerRootDir")])
    return Path(output) if output else None


def checked_paths(worktree: Path, docker: Path | None) -> list[Path]:
    paths = [worktree]
    if docker is not None and docker.exists() and docker.stat().st_dev != worktree.stat().st_dev:
        paths.append(docker)
    return paths


def short_mounts(paths: list[Path], floor_gb: int, measure: FreeBytes) -> list[Mount]:
    mounts = [Mount(path, measure(path)) for path in paths]
    return [mount for mount in mounts if mount.free_bytes < floor_gb * BYTES_PER_GB]


def target_directories(root: Path) -> list[Path]:
    found = []
    for depth in range(1, TARGET_SEARCH_DEPTH + 1):
        pattern = "/".join(["*"] * (depth - 1) + ["target"])
        found.extend(path for path in root.glob(pattern) if path.is_dir())
    return found


def largest_targets(roots: list[Path], command: Command) -> list[str]:
    sizes = []
    for directory in {path for root in roots for path in target_directories(root)}:
        output = command(["du", "-sb", str(directory)])
        if output:
            sizes.append((int(output.split()[0]), directory))
    sizes.sort(reverse=True)
    return [f"  {size / BYTES_PER_GB:.1f} GB {path}" for size, path in sizes[:TARGET_HINTS]]


def docker_reclaimable(command: Command) -> list[str]:
    output = command(
        [
            "docker",
            "system",
            "df",
            "--format",
            f"{go_template('.Type')}: {go_template('.Reclaimable')}",
        ]
    )
    return [f"  {line}" for line in output.splitlines()] if output else []


def shortage_message(
    mounts: list[Mount], floor_gb: int, targets: list[str], reclaimable: list[str]
) -> str:
    lines = [
        f"gate: {mount.path} has {mount.free_bytes / BYTES_PER_GB:.1f} GB free, "
        f"below the {floor_gb} GB floor"
        for mount in mounts
    ]
    lines.append("gate: free space before you run the gate; nothing was deleted.")
    if targets:
        lines.append("gate: largest target/ directories:")
        lines.extend(targets)
    if reclaimable:
        lines.append("gate: Docker reclaimable space (docker system df):")
        lines.extend(reclaimable)
    lines.append("gate: [quality.disk] floor_gb and step_floor_gb set the floor.")
    return "\n".join(lines)


def disk_error(
    settings: dict,
    step_names: Iterable[str],
    worktree: Path,
    environment: dict[str, str],
    probe: Probe | None = None,
) -> str | None:
    probe = probe or Probe()
    command = probe.command
    floor_gb = required_floor_gb(settings, step_names, environment)
    if floor_gb <= 0:
        return None
    paths = checked_paths(worktree, docker_root(command))
    mounts = short_mounts(paths, floor_gb, probe.measure)
    if not mounts:
        return None
    roots = [worktree, worktree.parent]
    targets = largest_targets(roots, command)
    return shortage_message(mounts, floor_gb, targets, docker_reclaimable(command))
