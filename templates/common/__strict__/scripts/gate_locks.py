"""Host-wide resource locks and a shared core pool for concurrent gates.

`BAUKIT_GATE_LOCK_DIR` holds one `<resource>.lock` file per host-exclusive resource.
`BAUKIT_GATE_CORE_POOL` holds `slot-<n>.lock` files, one per core. Both rely on flock, so
the kernel frees them when a gate dies. File descriptors stay non-inheritable, so no child
of a step keeps a lock alive.
"""

from __future__ import annotations

import fcntl
import os
import threading
import time
import uuid
from collections.abc import Iterator, Mapping
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from pathlib import Path

LOCK_DIR_ENV = "BAUKIT_GATE_LOCK_DIR"
CORE_POOL_ENV = "BAUKIT_GATE_CORE_POOL"
RESOURCE_ORDER = ("android", "browser")
IMPLIED_RESOURCES = {"android": ("browser",)}
DEFAULT_RESOURCES = {
    "android": ("android-boot", "android-e2e", "android-debug"),
    "browser": ("playwright", "webkit-repeats"),
}
POOL_POLL_SECONDS = 0.2
LOCK_POLL_SECONDS = 0.05
TICKET_READ_BYTES = 4096
LOCK_MODE = 0o600


@dataclass(frozen=True)
class LockWait:
    resource: str
    seconds: int


def resource_map(settings: Mapping) -> dict[str, frozenset[str]]:
    """Merge the template defaults with `[quality.resources]` from baukit.toml."""
    merged = {name: set(members) for name, members in DEFAULT_RESOURCES.items()}
    configured = settings.get("quality", {}).get("resources", {})
    for name, members in configured.items():
        merged.setdefault(name, set()).update(members)
    return {name: frozenset(members) for name, members in merged.items()}


def resource_rank(name: str) -> tuple[int, str]:
    known = RESOURCE_ORDER.index(name) if name in RESOURCE_ORDER else len(RESOURCE_ORDER)
    return known, name


def resources_for(names: set[str], resources: Mapping[str, frozenset[str]]) -> list[str]:
    """Return the resources a step with these names (step name and lane) needs, in lock order."""
    needed = {name for name, members in resources.items() if members & names}
    for name in list(needed):
        needed.update(IMPLIED_RESOURCES.get(name, ()))
    return sorted(needed, key=resource_rank)


def unknown_resource_members(settings: Mapping, lanes_and_steps: set[str]) -> list[str]:
    configured = settings.get("quality", {}).get("resources", {})
    return sorted(
        {member for members in configured.values() for member in members} - lanes_and_steps
    )


def open_locked(path: Path) -> int:
    path.parent.mkdir(parents=True, exist_ok=True)
    return os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, LOCK_MODE)


class ResourceLocks:
    """Fair, all-or-nothing resource locks.

    A step queues a ticket naming every resource it needs. It tries the resource locks, all
    in one pass without blocking, only when no earlier live ticket names one of them. It
    never holds one lock while it waits for another, and a later step cannot overtake an
    earlier step that wants the same resource, so the wait is bounded by the holders ahead.
    """

    def __init__(self, directory: Path):
        self.directory = directory
        self.queue = directory / "queue"

    @classmethod
    def from_environment(cls, environment: Mapping[str, str]) -> ResourceLocks | None:
        directory = environment.get(LOCK_DIR_ENV)
        return cls(Path(directory)) if directory else None

    def enqueue(self, resources: list[str]) -> tuple[int, Path]:
        self.queue.mkdir(parents=True, exist_ok=True)
        unique = uuid.uuid4().hex
        pending = self.queue / f"{unique}.pending"
        descriptor = os.open(pending, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, LOCK_MODE)
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        os.write(descriptor, "\n".join(resources).encode())
        with ordering_lock(self.directory):
            ticket = self.queue / f"{time.time_ns():020d}-{unique}.ticket"
            os.rename(pending, ticket)
        return descriptor, ticket

    def blocked_by_earlier(self, ticket: Path, resources: list[str]) -> bool:
        for other in sorted(self.queue.glob("*.ticket")):
            if other.name >= ticket.name:
                break
            wanted = live_ticket_resources(other)
            if wanted and wanted & set(resources):
                return True
        return False

    def try_take(self, resources: list[str], stack: ExitStack) -> bool:
        taken: list[int] = []
        for resource in resources:
            descriptor = open_locked(self.directory / f"{resource}.lock")
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                os.close(descriptor)
                for held in taken:
                    os.close(held)
                return False
            taken.append(descriptor)
        for descriptor in taken:
            stack.callback(os.close, descriptor)
        return True

    @contextmanager
    def hold(self, resources: list[str]) -> Iterator[list[LockWait]]:
        started = time.monotonic()
        descriptor, ticket = self.enqueue(resources)
        with ExitStack() as stack:
            try:
                while self.blocked_by_earlier(ticket, resources) or not self.try_take(
                    resources, stack
                ):
                    time.sleep(LOCK_POLL_SECONDS)
            finally:
                ticket.unlink(missing_ok=True)
                os.close(descriptor)
            waited = round(time.monotonic() - started)
            yield [LockWait(resource, waited) for resource in resources]


@contextmanager
def ordering_lock(directory: Path) -> Iterator[None]:
    descriptor = open_locked(directory / "queue.lock")
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        os.close(descriptor)


def live_ticket_resources(ticket: Path) -> set[str]:
    """Return the resources of a ticket whose owner is alive, and drop dead tickets."""
    try:
        descriptor = os.open(ticket, os.O_RDWR | os.O_CLOEXEC)
    except FileNotFoundError:
        return set()
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return set(os.read(descriptor, TICKET_READ_BYTES).decode().split())
        ticket.unlink(missing_ok=True)
        return set()
    finally:
        os.close(descriptor)


def describe_waits(waits: list[LockWait]) -> str:
    return ", ".join(f"waited {wait.seconds} s for {wait.resource}.lock" for wait in waits)


class CorePool:
    """Slot files that concurrent gates take with non-blocking flock."""

    def __init__(self, directory: Path, size: int):
        self.directory = directory
        self.size = size
        self.held: list[int] = []
        self.guard = threading.Lock()
        self.registration = self.register()

    @classmethod
    def from_environment(cls, environment: Mapping[str, str], size: int) -> CorePool | None:
        directory = environment.get(CORE_POOL_ENV)
        return cls(Path(directory), size) if directory else None

    def register(self) -> int:
        name = f"{os.getpid()}-{uuid.uuid4().hex}.gate"
        with ordering_lock(self.directory):
            descriptor = open_locked(self.directory / "gates" / name)
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        return descriptor

    def live_gates(self) -> int:
        """Count the gates that registered in this pool and still run, this one included."""
        live = 0
        with ordering_lock(self.directory):
            for gate in (self.directory / "gates").glob("*.gate"):
                live += self.is_running(gate)
        return max(1, live)

    @staticmethod
    def is_running(gate: Path) -> bool:
        descriptor = os.open(gate, os.O_RDWR | os.O_CLOEXEC)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        finally:
            os.close(descriptor)
        gate.unlink(missing_ok=True)
        return False

    def slot(self, number: int) -> Path:
        return self.directory / f"slot-{number}.lock"

    def try_take(self, want: int) -> list[int]:
        taken: list[int] = []
        for number in range(self.size):
            if len(taken) == want:
                break
            descriptor = open_locked(self.slot(number))
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                os.close(descriptor)
                continue
            taken.append(descriptor)
        return taken

    def acquire(self, want: int, minimum: int) -> int:
        while True:
            taken = self.try_take(want)
            if len(taken) >= minimum:
                with self.guard:
                    self.held.extend(taken)
                return len(taken)
            self.close(taken)
            time.sleep(POOL_POLL_SECONDS)

    def release(self, cores: int) -> None:
        with self.guard:
            released = [self.held.pop() for _ in range(min(cores, len(self.held)))]
        self.close(released)

    def available(self) -> int:
        probe = self.try_take(self.size)
        self.close(probe)
        return len(probe)

    @staticmethod
    def close(descriptors: list[int]) -> None:
        for descriptor in descriptors:
            os.close(descriptor)
