"""Host-wide resource locks and a shared core pool for concurrent gates.

`BAUKIT_GATE_LOCK_DIR` holds one `<resource>.lock` file per host resource. A step locks it
exclusively, or shared with other shared holders.
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
RESOURCE_ORDER = ("android", "docker-network")
DEFAULT_RESOURCES = {
    "android": ("android-boot", "android-e2e", "android-debug"),
    "docker-network": ("android-e2e",),
}
SHARED_SUFFIX = "~shared"
POOL_POLL_SECONDS = 0.2
LOCK_POLL_SECONDS = 0.05
TICKET_READ_BYTES = 4096
LOCK_MODE = 0o600


@dataclass(frozen=True)
class Need:
    """A resource a step locks, exclusively or shared with other shared holders."""

    resource: str
    shared: bool = False

    @property
    def token(self) -> str:
        return self.resource + (SHARED_SUFFIX if self.shared else "")

    @classmethod
    def parse(cls, token: str) -> Need:
        shared = token.endswith(SHARED_SUFFIX)
        return cls(token.removesuffix(SHARED_SUFFIX), shared)

    def conflicts_with(self, other: Need) -> bool:
        return self.resource == other.resource and not (self.shared and other.shared)


@dataclass(frozen=True)
class LockWait:
    resource: str
    seconds: int


@dataclass(frozen=True)
class ResourceRule:
    name: str
    members: frozenset[str]
    shared: bool = False


def resource_map(settings: Mapping) -> list[ResourceRule]:
    """Merge the template defaults with `[quality.resources]` and `[quality.shared_resources]`."""
    quality = settings.get("quality", {})
    exclusive = {name: set(members) for name, members in DEFAULT_RESOURCES.items()}
    for name, members in quality.get("resources", {}).items():
        exclusive.setdefault(name, set()).update(members)
    rules = [ResourceRule(name, frozenset(members)) for name, members in exclusive.items()]
    for name, members in quality.get("shared_resources", {}).items():
        rules.append(ResourceRule(name, frozenset(members), shared=True))
    return rules


def resource_rank(need: Need) -> tuple[int, str]:
    known = (
        RESOURCE_ORDER.index(need.resource)
        if need.resource in RESOURCE_ORDER
        else len(RESOURCE_ORDER)
    )
    return known, need.resource


def resources_for(names: set[str], rules: list[ResourceRule]) -> list[Need]:
    """Return what a step with these names (step name and lane) locks, in lock order.

    A step that matches a resource both ways locks it exclusively.
    """
    needed: dict[str, Need] = {}
    for rule in rules:
        if rule.members & names and rule.name not in needed:
            needed[rule.name] = Need(rule.name, rule.shared)
        elif rule.members & names and not rule.shared:
            needed[rule.name] = Need(rule.name)
    return sorted(needed.values(), key=resource_rank)


def unknown_resource_members(settings: Mapping, lanes_and_steps: set[str]) -> list[str]:
    quality = settings.get("quality", {})
    configured = [
        *quality.get("resources", {}).values(),
        *quality.get("shared_resources", {}).values(),
    ]
    return sorted({member for members in configured for member in members} - lanes_and_steps)


def open_locked(path: Path) -> int:
    path.parent.mkdir(parents=True, exist_ok=True)
    return os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, LOCK_MODE)


class ResourceLocks:
    """Fair, all-or-nothing resource locks.

    A step queues a ticket naming every resource it needs and how. It tries the resource
    locks, all in one pass without blocking, only when no earlier live ticket conflicts with
    it. Two tickets conflict when they name one resource and at least one wants it
    exclusively. It never holds one lock while it waits for another, and a later step cannot
    overtake an earlier conflicting step, so shared arrivals cannot starve an exclusive
    waiter and the wait is bounded by the holders ahead.
    """

    def __init__(self, directory: Path):
        self.directory = directory
        self.queue = directory / "queue"

    @classmethod
    def from_environment(cls, environment: Mapping[str, str]) -> ResourceLocks | None:
        directory = environment.get(LOCK_DIR_ENV)
        return cls(Path(directory)) if directory else None

    def enqueue(self, resources: list[Need]) -> tuple[int, Path]:
        self.queue.mkdir(parents=True, exist_ok=True)
        unique = uuid.uuid4().hex
        pending = self.queue / f"{unique}.pending"
        descriptor = os.open(pending, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, LOCK_MODE)
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        os.write(descriptor, "\n".join(need.token for need in resources).encode())
        with ordering_lock(self.directory):
            ticket = self.queue / f"{time.time_ns():020d}-{unique}.ticket"
            os.rename(pending, ticket)
        return descriptor, ticket

    def blocked_by_earlier(self, ticket: Path, resources: list[Need]) -> bool:
        for other in sorted(self.queue.glob("*.ticket")):
            if other.name >= ticket.name:
                break
            wanted = live_ticket_resources(other)
            if any(mine.conflicts_with(theirs) for mine in resources for theirs in wanted):
                return True
        return False

    def try_take(self, resources: list[Need], stack: ExitStack) -> bool:
        taken: list[int] = []
        for need in resources:
            descriptor = open_locked(self.directory / f"{need.resource}.lock")
            mode = fcntl.LOCK_SH if need.shared else fcntl.LOCK_EX
            try:
                fcntl.flock(descriptor, mode | fcntl.LOCK_NB)
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
    def hold(self, resources: list[Need]) -> Iterator[list[LockWait]]:
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
            yield [LockWait(need.resource, waited) for need in resources]


@contextmanager
def ordering_lock(directory: Path) -> Iterator[None]:
    descriptor = open_locked(directory / "queue.lock")
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        os.close(descriptor)


def live_ticket_resources(ticket: Path) -> set[Need]:
    """Return the resources of a ticket whose owner is alive, and drop dead tickets."""
    try:
        descriptor = os.open(ticket, os.O_RDWR | os.O_CLOEXEC)
    except FileNotFoundError:
        return set()
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            text = os.read(descriptor, TICKET_READ_BYTES).decode()
            return {Need.parse(token) for token in text.split()}
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
