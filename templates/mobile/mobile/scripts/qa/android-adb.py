import hashlib
import math
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

DEFAULT_PROBE_TIMEOUT = 10
DEFAULT_COMMAND_TIMEOUT = 60
ADB_TIMEOUT_SETTING = "BAUKIT_QA_ADB_TIMEOUT_SECONDS"
DEFAULT_BOOT_TIMEOUT = 360
DEFAULT_IDLE_TIMEOUT = 180
DEFAULT_SETTLE_TIMEOUT = 600
DEFAULT_STOP_TIMEOUT = 120
DEFAULT_WIFI_TIMEOUT = 90
DEFAULT_LOAD_WAIT_TIMEOUT = 600
DEFAULT_MAX_LOAD_PER_CORE = 2.0
LOAD_POLL_INTERVAL = 15
HOST_ALIAS = "10.0.2.2"
LOOPBACK = "127.0.0.1"
CONSOLE_CONNECT_TIMEOUT = 1.0
SERIAL_PATTERN = re.compile(r"^emulator-(\d+)$")
WIFI_STATUS_LINES = 4
SETTLE_WINDOW_POLLS = 5
QUIET_CORES = 1.0
SNAPSHOT_PREFIX = "baukit-clean"
SNAPSHOT_HASH_LENGTH = 12
IDLE_STABLE_PROBES = 3
MIN_EVENT_FIELDS = 3
HEALTH_BASE_ARGUMENTS = 4
BOOT_BASE_ARGUMENTS = 4
POLL_INTERVAL = 2
READY_STATE = "device"
FOREIGN_UNREADY_STATUS = 3
MAESTRO_LISTING = (
    "Maestro reports every Android device as not connected while any adb transport is not ready"
)
SYSTEMUI = "com.android.systemui"
ANR_DIALOG = "Not Responding"
CLOSE_SYSTEM_DIALOGS = "android.intent.action.CLOSE_SYSTEM_DIALOGS"
CRITICAL_PACKAGES = {
    SYSTEMUI,
    "com.android.inputmethod.latin",
    "com.google.android.inputmethod.latin",
}


@dataclass(frozen=True)
class Timing:
    clock: Callable[[], float] = time.monotonic
    sleep: Callable[[float], None] = time.sleep


SYSTEM_TIMING = Timing()


@dataclass(frozen=True)
class Emulator:
    serial: str
    pid: int | None = None


def run_command(command: list[str], **options) -> subprocess.CompletedProcess:
    return subprocess.run(  # noqa: S603 -- fixed adb program or the caller's argv list, no shell
        command, check=False, **options
    )


def timeout_setting(name: str, default: int) -> float:
    value = float(os.environ.get(name, default))
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name} must be a positive finite number")
    return value


def process_running(pid: int) -> bool:
    result = run_command(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True)
    state = result.stdout.strip()
    return result.returncode == 0 and bool(state) and not state.startswith("Z")


def wait_for_boot(
    adb: str, serial: str, probe_timeout: float, boot_timeout: float, pid: int | None = None
) -> int:
    deadline = time.monotonic() + boot_timeout
    while (remaining := deadline - time.monotonic()) > 0:
        exited = pid is not None and not process_running(pid)
        try:
            result = run_command(
                [adb, "-s", serial, "shell", "getprop", "sys.boot_completed"],
                capture_output=True,
                text=True,
                timeout=min(probe_timeout, remaining),
            )
            if result.returncode == 0 and result.stdout.strip() == "1":
                return 0
        except subprocess.TimeoutExpired:
            print("qa: Android boot probe timed out", file=sys.stderr)
        if exited:
            print(f"qa: emulator process {pid} exited before {serial} booted", file=sys.stderr)
            return 1
        time.sleep(min(poll_interval(), max(0, deadline - time.monotonic())))
    return 124


def shell(adb: str, serial: str, timeout: float, *command: str) -> str | None:
    try:
        result = run_command(
            [adb, "-s", serial, "shell", *command],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return None
    return result.stdout if result.returncode == 0 else None


@dataclass(frozen=True)
class IdleProbe:
    pid: str = ""
    focus: str = ""

    @property
    def anr_dialog(self) -> bool:
        return ANR_DIALOG in self.focus

    @property
    def idle(self) -> bool:
        focused = bool(self.focus) and "null" not in self.focus
        return bool(self.pid) and focused and not self.anr_dialog

    def describe(self) -> str:
        return f"systemui pid {self.pid or 'none'}, {self.focus or 'no mCurrentFocus line'}"


def systemui_idle_probe(adb: str, serial: str, timeout: float) -> IdleProbe:
    pid = (shell(adb, serial, timeout, "pidof", SYSTEMUI) or "").strip()
    windows = shell(adb, serial, timeout, "dumpsys", "window", "displays") or ""
    focus = next((line.strip() for line in windows.splitlines() if "mCurrentFocus=" in line), "")
    return IdleProbe(pid, focus)


def close_anr_dialog(adb: str, serial: str, timeout: float, probe: IdleProbe) -> None:
    """A quick-boot snapshot keeps an ANR dialog that was open when it was saved, and only
    CLOSE_SYSTEM_DIALOGS closes it; hide_error_dialogs and KEYCODE_BACK leave it focused."""
    print(f"qa: closing an ANR dialog on {serial}: {probe.focus}", file=sys.stderr)
    shell(adb, serial, timeout, "am", "broadcast", "-a", CLOSE_SYSTEM_DIALOGS)


def poll_interval() -> float:
    return float(os.environ.get("BAUKIT_QA_POLL_SECONDS", POLL_INTERVAL))


def wait_for_wifi_route(adb: str, serial: str, probe_timeout: float, wifi_timeout: float) -> int:
    started = time.monotonic()
    deadline = started + wifi_timeout
    route = ""
    while deadline - time.monotonic() > 0:
        route = shell(adb, serial, probe_timeout, "ip", "-4", "route", "get", HOST_ALIAS) or ""
        if " dev wlan0 " in f" {route} ":
            print(
                f"qa: {serial} has a Wi-Fi route after {round(time.monotonic() - started)} s",
                file=sys.stderr,
            )
            return 0
        time.sleep(min(poll_interval(), max(0, deadline - time.monotonic())))
    status = shell(adb, serial, probe_timeout, "cmd", "wifi", "status") or ""
    print(
        f"qa: {serial} has no Wi-Fi route to {HOST_ALIAS}; offline flows need Wi-Fi",
        file=sys.stderr,
    )
    print(f"qa: last route: {route.strip() or 'none'}", file=sys.stderr)
    print(
        f"qa: Wi-Fi status: {wifi_summary(status)}",
        file=sys.stderr,
    )
    return 124


def wait_for_idle(
    adb: str,
    serial: str,
    probe_timeout: float,
    idle_timeout: float,
    *,
    timing: Timing = SYSTEM_TIMING,
) -> int:
    deadline = timing.clock() + idle_timeout
    stable = 0
    last_pid = None
    probe = IdleProbe()
    while deadline - timing.clock() > 0:
        probe = systemui_idle_probe(adb, serial, probe_timeout)
        if probe.anr_dialog:
            close_anr_dialog(adb, serial, probe_timeout, probe)
        pid = probe.pid if probe.idle else None
        stable = stable + 1 if pid is not None and pid == last_pid else 0
        last_pid = pid
        if stable >= IDLE_STABLE_PROBES - 1:
            return 0
        timing.sleep(min(poll_interval(), max(0, deadline - timing.clock())))
    print("qa: systemui did not settle before the idle deadline", file=sys.stderr)
    print(f"qa: last idle probe: {probe.describe()}", file=sys.stderr)
    return 124


def cpu_counters(stat: str) -> tuple[int, int, int] | None:
    lines = stat.splitlines()
    total_line = next((line.split() for line in lines if line.startswith("cpu ")), None)
    cores = sum(1 for line in lines if re.match(r"cpu\d+ ", line))
    if total_line is None or not cores:
        return None
    values = [int(value) for value in total_line[1:]]
    idle = sum(values[3:5])
    return sum(values), sum(values) - idle, cores


def busy_cores(before: tuple[int, int, int], after: tuple[int, int, int]) -> float:
    total = after[0] - before[0]
    return 0.0 if total <= 0 else (after[1] - before[1]) / total * after[2]


def settle(
    adb: str,
    serial: str,
    probe_timeout: float,
    settle_timeout: float,
    *,
    timing: Timing = SYSTEM_TIMING,
) -> int:
    clock = timing.clock
    start = clock()
    deadline = start + settle_timeout
    shell(
        adb,
        serial,
        max(probe_timeout, deadline - clock()),
        "cmd",
        "package",
        "bg-dexopt-job",
    )
    window = SETTLE_WINDOW_POLLS * poll_interval()
    before = None
    while clock() < deadline:
        stat = shell(adb, serial, probe_timeout, "cat", "/proc/stat")
        after = cpu_counters(stat) if stat else None
        if after is None:
            if attached(adb, serial, probe_timeout):
                print(f"qa: lost {serial} while it settled", file=sys.stderr)
                return 1
        elif before is not None and busy_cores(before, after) < QUIET_CORES:
            print(f"qa: {serial} settled in {clock() - start:.0f} s")
            return 0
        before = after
        timing.sleep(min(window, max(0, deadline - clock())))
    print(f"qa: {serial} did not settle within {settle_timeout:.0f} s; saving the snapshot anyway")
    return 0


def snapshot_name(config: Path, sdk_root: Path, gpu: str, features: str, schema: str) -> str:
    settings = {}
    for line in (config.read_text() if config.is_file() else "").splitlines():
        key, separator, value = line.partition("=")
        if separator:
            settings[key.strip()] = value.strip()
    image = settings.get("image.sysdir.1", "").strip("/")
    properties = sdk_root / image / "source.properties"
    digest = hashlib.sha256()
    for part in (
        image,
        properties.read_text() if properties.is_file() else "",
        *sorted(f"{key}={value}" for key, value in settings.items() if key.startswith("hw.")),
        gpu,
        features,
        schema,
    ):
        digest.update(part.encode() + b"\0")
    return f"{SNAPSHOT_PREFIX}-{digest.hexdigest()[:SNAPSHOT_HASH_LENGTH]}"


def loaded_snapshots(avd_name: str, proc: Path = Path("/proc")) -> set[str]:
    return {
        pairs["-snapshot"]
        for pairs in process_arguments(proc)
        if pairs.get("-avd") == avd_name and "-snapshot" in pairs
    }


def process_arguments(proc: Path = Path("/proc")) -> list[dict[str, str]]:
    running = []
    for cmdline in proc.glob("[0-9]*/cmdline"):
        try:
            args = cmdline.read_bytes().decode(errors="replace").split("\0")
        except OSError:
            continue
        running.append(dict(zip(args, args[1:], strict=False)))
    return running


def process_avd_home(environ: Path) -> str | None:
    try:
        variables = environ.read_bytes().decode(errors="replace").split("\0")
    except OSError:
        return None
    homes = [entry.split("=", 1)[1] for entry in variables if entry.startswith("ANDROID_AVD_HOME=")]
    return homes[0] if homes else None


def avd_running(avd_dir: Path, proc: Path = Path("/proc")) -> bool:
    """Whether an emulator runs this AVD; one whose AVD home is unreadable counts as running."""
    name = avd_dir.name.removesuffix(".avd")
    for cmdline in proc.glob("[0-9]*/cmdline"):
        try:
            args = cmdline.read_bytes().decode(errors="replace").split("\0")
        except OSError:
            continue
        if dict(zip(args, args[1:], strict=False)).get("-avd") != name:
            continue
        home = process_avd_home(cmdline.with_name("environ"))
        if home is None or Path(home) == avd_dir.parent:
            return True
    return False


# The emulator rebuilds the cache partition when version_num.cache is missing.
SEED_SUFFIXES = (".img", ".img.qcow2", "version_num.cache")


def retarget_hardware(hardware: Path, base_avd: str, clone_avd: str) -> None:
    """Point a copied snapshot's hardware.ini at the clone; the emulator refuses a mismatch."""
    if not hardware.is_file():
        return
    clone_name = clone_avd.removesuffix(".avd")
    identity = {"avd.name", "avd.id"}
    lines = [
        f"{key} = {clone_name}"
        if (key := line.partition(" = ")[0]) in identity
        else line.replace(f"/{base_avd}/", f"/{clone_avd}/")
        for line in hardware.read_text().splitlines()
    ]
    hardware.write_text("\n".join(lines) + "\n")


def seed_clone(base_dir: Path, clone_dir: Path, snapshot: str, running: bool) -> int:
    """Copy the base AVD's disks and quick-boot snapshot into an extra AVD that has none.

    The qcow2 overlays name their backing files relative to the AVD directory, so the copies
    resolve to the clone's own images. A running base emulator writes its overlays, so the
    copy waits for a stopped base.
    """
    source = base_dir / "snapshots" / snapshot
    if running or not source.is_dir() or not (base_dir / f"{snapshot}.ready").is_file():
        return 1
    target = clone_dir / "snapshots" / snapshot
    shutil.rmtree(target, ignore_errors=True)
    for image in base_dir.iterdir():
        if image.is_file() and image.name.endswith(SEED_SUFFIXES):
            shutil.copy2(image, clone_dir / image.name)
    shutil.copytree(source, target)
    retarget_hardware(target / "hardware.ini", base_dir.name, clone_dir.name)
    (clone_dir / f"{snapshot}.ready").touch()
    prune_snapshots(clone_dir, snapshot, loaded_snapshots(clone_dir.name.removesuffix(".avd")))
    print(f"qa: seeded {clone_dir.name} from the {snapshot} snapshot of {base_dir.name}")
    return 0


def seed_clone_mode(_probe_timeout: float) -> int:
    base_dir, clone_dir, snapshot = Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4]
    return seed_clone(base_dir, clone_dir, snapshot, avd_running(base_dir))


def prune_snapshots(avd_dir: Path, keep: str, loaded: set[str]) -> int:
    """Delete every other baukit snapshot of this AVD except ones a running emulator loaded."""
    snapshots = avd_dir / "snapshots"
    older = [
        entry
        for entry in (snapshots.iterdir() if snapshots.is_dir() else ())
        if entry.name.startswith(SNAPSHOT_PREFIX) and entry.name != keep and entry.is_dir()
    ]
    kept = {keep, *loaded}
    for entry in older:
        if entry.name in kept:
            print(f"qa: kept older quick-boot snapshot {entry.name} that a running emulator loaded")
            continue
        shutil.rmtree(entry)
        print(f"qa: removed stale quick-boot snapshot {entry.name}")
    for ready in avd_dir.glob(f"{SNAPSHOT_PREFIX}*.ready"):
        if ready.name.removesuffix(".ready") not in kept:
            ready.unlink()
    return 0


def stale_snapshot_tags(listing: str, avd_dir: Path, keep: str, loaded: set[str]) -> list[str]:
    """Names of baukit qcow2 snapshot tags whose snapshot directory was pruned.

    `listing` is `emu avd snapshot list` output. Deleting only the directory leaves the tag and
    its VM state in every qcow2 overlay, and a read-only launch copies those overlays.
    """
    tags = {fields[1] for line in listing.splitlines() if len(fields := line.split()) > 1}
    return sorted(
        tag
        for tag in tags
        if tag.startswith(SNAPSHOT_PREFIX)
        and tag not in {keep, *loaded}
        and not (avd_dir / "snapshots" / tag).is_dir()
    )


def attached(adb: str, serial: str, timeout: float) -> int:
    try:
        result = run_command(
            [adb, "-s", serial, "get-state"],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return 124
    return 0 if result.returncode == 0 and result.stdout.strip() == "device" else 1


@dataclass(frozen=True)
class MaestroTargets:
    """Serials one Maestro run requests, how long they may be off adb, and how long others may."""

    serials: tuple[str, ...]
    grace: float
    foreign_limit: float


def transport_states(adb: str, timeout: float) -> dict[str, str] | None:
    try:
        result = run_command([adb, "devices"], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None
    if result.returncode != 0:
        return None
    states = {}
    for line in result.stdout.splitlines():
        serial, separator, state = line.partition("\t")
        if separator:
            states[serial] = state.strip()
    return states


def unready_transports(
    states: dict[str, str] | None, serials: tuple[str, ...]
) -> tuple[list[str], list[str]]:
    if states is None:
        return ["adb devices did not answer"], []
    own = [
        f"{serial} is {states.get(serial, 'not listed')} on adb"
        for serial in serials
        if states.get(serial) != READY_STATE
    ]
    foreign = [
        f"{serial} ({state}, not this run's)"
        for serial, state in sorted(states.items())
        if serial not in serials and state != READY_STATE
    ]
    return own, foreign


def await_maestro_devices(
    adb: str, targets: MaestroTargets, probe_timeout: float, *, timing: Timing = SYSTEM_TIMING
) -> int:
    """Waits until every transport the shared adb server lists is in state `device`.

    Maestro 2.8.0 opens a transport to every serial adb lists. One that is offline, for example
    another session's emulator while it boots or stops, makes it report every Android device as
    not connected. The run's own serials may be off adb for `grace`, other transports for
    `foreign_limit`.
    """
    started = timing.clock()
    own_since = None
    announced = False
    while True:
        own, foreign = unready_transports(transport_states(adb, probe_timeout), targets.serials)
        if not own and not foreign:
            return 0
        now = timing.clock()
        if not own:
            own_since = None
        elif own_since is None:
            own_since = now
        limit, elapsed = (
            (targets.grace, now - own_since)
            if own_since is not None
            else (targets.foreign_limit, now - started)
        )
        if elapsed >= limit:
            break
        if foreign and not announced:
            print(
                f"qa: waiting for {', '.join(foreign)} on adb; {MAESTRO_LISTING}", file=sys.stderr
            )
            announced = True
        timing.sleep(min(poll_interval(), limit - elapsed))
    for problem in own:
        print(f"qa: {problem} after {limit:g} s; Maestro needs state device", file=sys.stderr)
    if own:
        return 1
    print(
        f"qa: adb still lists {', '.join(foreign)} after {limit:g} s; {MAESTRO_LISTING}",
        file=sys.stderr,
    )
    return FOREIGN_UNREADY_STATUS


def report_unready_transports(adb: str, serials: tuple[str, ...], probe_timeout: float) -> int:
    own, foreign = unready_transports(transport_states(adb, probe_timeout), serials)
    if own or foreign:
        print(f"qa: adb now reports {', '.join(own + foreign)}; {MAESTRO_LISTING}", file=sys.stderr)
    return 0


def console_port_open(serial: str) -> bool:
    match = SERIAL_PATTERN.match(serial)
    if match is None:
        return False
    try:
        with socket.create_connection((LOOPBACK, int(match[1])), timeout=CONSOLE_CONNECT_TIMEOUT):
            return True
    except OSError:
        return False


def adb_lists(adb: str, serial: str, timeout: float) -> bool | None:
    try:
        result = run_command([adb, "devices"], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None
    if result.returncode != 0:
        print("qa: Android shutdown probe failed", file=sys.stderr)
        raise ChildProcessError(result.returncode)
    return any(line.split()[:1] == [serial] for line in result.stdout.splitlines())


def wait_for_stop(
    adb: str,
    emulator: Emulator,
    probe_timeout: float,
    stop_timeout: float,
    *,
    timing: Timing = SYSTEM_TIMING,
) -> int:
    """Waits until adb forgot the serial, its console port closed and the qemu process exited.

    qemu closes its console about a second before it exits and may take up to 20 s to
    shut down, still holding the AVD's files, so the next emulator waits for `pid` to
    exit. A probe that times out under load is retried until the stop deadline.
    """
    serial, pid = emulator.serial, emulator.pid
    started = timing.clock()
    deadline = started + stop_timeout
    listed: bool | None = None
    console = running = False
    while (remaining := deadline - timing.clock()) > 0:
        try:
            listed = adb_lists(adb, serial, min(probe_timeout, remaining))
        except ChildProcessError as failure:
            return int(failure.args[0])
        console = listed is False and console_port_open(serial)
        running = listed is False and not console and pid is not None and process_running(pid)
        if listed is False and not console and not running:
            print(f"qa: {serial} stopped after {timing.clock() - started:.1f} s", file=sys.stderr)
            return 0
        timing.sleep(min(poll_interval(), max(0, deadline - timing.clock())))
    state = "adb probes timed out" if listed is None else "still listed by adb"
    if console:
        state = "still holding its console port"
    elif running:
        state = f"qemu process {pid} still running"
    print(f"qa: {serial} did not stop within {stop_timeout:g} s ({state})", file=sys.stderr)
    return 124


def health_result(status: int, serial: str, lines: list[str], report: Path | None) -> int:
    if status:
        print("\n".join(lines), file=sys.stderr)
    if report is not None:
        report.parent.mkdir(parents=True, exist_ok=True)
        with report.open("a") as output:
            output.write(
                f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {serial} status={status}\n"
            )
            output.write("\n".join(lines) + "\n")
    return status


def anr_details(adb: str, serial: str, timeout: float) -> tuple[list[str], set[str]]:
    try:
        result = run_command(
            [adb, "-s", serial, "shell", "dumpsys", "activity", "lastanr"],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return ["qa: Android ANR diagnostics timed out"], set()
    if result.returncode != 0:
        return [f"qa: Android ANR diagnostics failed: exit {result.returncode}"], set()
    prefixes = ("ANR time:", "ANR in ", "PID:", "Reason:", "ErrorId:", "Frozen:")
    lines = [line for line in result.stdout.splitlines() if line.strip().startswith(prefixes)]
    packages = {
        package
        for package in CRITICAL_PACKAGES
        for line in lines
        if re.search(rf"(?<![\w.]){re.escape(package)}(?![\w.])", line)
    }
    return lines, packages


def check_health(adb: str, serial: str, timeout: float, report: Path | None = None) -> int:
    try:
        result = run_command(
            [adb, "-s", serial, "logcat", "-d", "-b", "events", "-s", "am_anr", "am_crash"],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return health_result(124, serial, ["qa: Android system health probe timed out"], report)
    if result.returncode != 0:
        return health_result(
            result.returncode, serial, ["qa: Android system health probe failed"], report
        )
    evidence = []
    anr_packages = set()
    for line in result.stdout.splitlines():
        fields = line.partition("[")[2].split(",")
        if len(fields) >= MIN_EVENT_FIELDS and fields[2].strip() in CRITICAL_PACKAGES:
            package = fields[2].strip()
            evidence.extend((f"qa: Android system process is unhealthy: {package}", line))
            if "am_anr" in line:
                anr_packages.add(package)
    details, dumpsys_packages = anr_details(adb, serial, timeout)
    if anr_packages and not anr_packages & dumpsys_packages:
        evidence.append(
            "qa: logcat am_anr shows the ANR above; dumpsys lastanr has no record of it"
        )
    for package in sorted(dumpsys_packages - anr_packages):
        evidence.append(f"qa: dumpsys lastanr reports an ANR in {package}")
    if dumpsys_packages or (anr_packages and details[:1] and details[0].startswith("qa: ")):
        evidence.extend(details)
    return health_result(int(bool(evidence)), serial, evidence, report)


def timed_mode(name: str, setting: str, default: float, action):
    def run(probe_timeout: float) -> int:
        return action(sys.argv[2], sys.argv[3], probe_timeout, timeout_setting(setting, default))

    return name, run


def wifi_summary(status: str) -> str:
    return " | ".join(status.strip().splitlines()[:WIFI_STATUS_LINES]) or "none"


def check_health_mode(probe_timeout: float) -> int:
    report = Path(sys.argv[4]) if len(sys.argv) > HEALTH_BASE_ARGUMENTS else None
    return check_health(sys.argv[2], sys.argv[3], probe_timeout, report)


def wait_for_boot_mode(probe_timeout: float) -> int:
    pid_argument = sys.argv[4] if len(sys.argv) > BOOT_BASE_ARGUMENTS else ""
    boot_timeout = timeout_setting("BAUKIT_QA_BOOT_TIMEOUT_SECONDS", DEFAULT_BOOT_TIMEOUT)
    pid = int(pid_argument) if pid_argument else None
    return wait_for_boot(sys.argv[2], sys.argv[3], probe_timeout, boot_timeout, pid)


def wait_for_stop_mode(probe_timeout: float) -> int:
    pid_argument = sys.argv[4] if len(sys.argv) > BOOT_BASE_ARGUMENTS else ""
    stop_timeout = timeout_setting("BAUKIT_QA_STOP_TIMEOUT_SECONDS", DEFAULT_STOP_TIMEOUT)
    pid = int(pid_argument) if pid_argument else None
    return wait_for_stop(sys.argv[2], Emulator(sys.argv[3], pid), probe_timeout, stop_timeout)


def snapshot_name_mode(_probe_timeout: float) -> int:
    print(
        snapshot_name(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4], sys.argv[5], sys.argv[6])
    )
    return 0


def prune_snapshots_mode(_probe_timeout: float) -> int:
    return prune_snapshots(Path(sys.argv[2]), sys.argv[3], loaded_snapshots(sys.argv[4]))


def stale_snapshot_tags_mode(_probe_timeout: float) -> int:
    avd_dir, keep, avd_name = Path(sys.argv[2]), sys.argv[3], sys.argv[4]
    for tag in stale_snapshot_tags(sys.stdin.read(), avd_dir, keep, loaded_snapshots(avd_name)):
        print(tag)
    return 0


def attached_mode(probe_timeout: float) -> int:
    return attached(sys.argv[2], sys.argv[3], probe_timeout)


def await_devices_mode(probe_timeout: float) -> int:
    foreign_limit = timeout_setting("BAUKIT_QA_BOOT_TIMEOUT_SECONDS", DEFAULT_BOOT_TIMEOUT)
    targets = MaestroTargets(tuple(sys.argv[4:]), float(sys.argv[3]), foreign_limit)
    return await_maestro_devices(sys.argv[2], targets, probe_timeout)


def report_unready_mode(probe_timeout: float) -> int:
    return report_unready_transports(sys.argv[2], tuple(sys.argv[3:]), probe_timeout)


def load_per_core() -> float:
    return os.getloadavg()[0] / (os.cpu_count() or 1)


def wait_for_load(
    limit: float,
    wait_timeout: float,
    read_load: Callable[[], float] = load_per_core,
    timing: Timing = SYSTEM_TIMING,
) -> int:
    deadline = timing.clock() + wait_timeout
    load = read_load()
    if load <= limit:
        return 0
    print(
        f"qa: host load is {load:.2f} per core,"
        f" above BAUKIT_QA_ANDROID_MAX_LOAD_PER_CORE={limit:g};"
        f" waiting up to {wait_timeout:g} s before starting emulators",
        file=sys.stderr,
    )
    while (remaining := deadline - timing.clock()) > 0:
        timing.sleep(min(LOAD_POLL_INTERVAL, remaining))
        load = read_load()
        if load <= limit:
            print(f"qa: host load fell to {load:.2f} per core", file=sys.stderr)
            return 0
    print(
        f"qa: host load stayed above {limit:g} per core for {wait_timeout:g} s (now {load:.2f});"
        " emulators under this load hit input ANRs, so none were started."
        " Retry when the host is quieter or raise BAUKIT_QA_ANDROID_MAX_LOAD_PER_CORE",
        file=sys.stderr,
    )
    return 1


def wait_for_load_mode(_probe_timeout: float) -> int:
    limit = timeout_setting("BAUKIT_QA_ANDROID_MAX_LOAD_PER_CORE", DEFAULT_MAX_LOAD_PER_CORE)
    wait_timeout = timeout_setting("BAUKIT_QA_ANDROID_LOAD_WAIT_SECONDS", DEFAULT_LOAD_WAIT_TIMEOUT)
    return wait_for_load(limit, wait_timeout)


MODES = dict(
    [
        ("--check-health", check_health_mode),
        timed_mode(
            "--wait-for-idle", "BAUKIT_QA_IDLE_TIMEOUT_SECONDS", DEFAULT_IDLE_TIMEOUT, wait_for_idle
        ),
        timed_mode("--settle", "BAUKIT_QA_SETTLE_TIMEOUT_SECONDS", DEFAULT_SETTLE_TIMEOUT, settle),
        ("--snapshot-name", snapshot_name_mode),
        ("--prune-snapshots", prune_snapshots_mode),
        ("--stale-snapshot-tags", stale_snapshot_tags_mode),
        timed_mode(
            "--wait-for-wifi-route",
            "BAUKIT_QA_WIFI_TIMEOUT_SECONDS",
            DEFAULT_WIFI_TIMEOUT,
            wait_for_wifi_route,
        ),
        ("--attached", attached_mode),
        ("--await-devices", await_devices_mode),
        ("--report-unready", report_unready_mode),
        ("--wait-for-stop", wait_for_stop_mode),
        ("--wait-for-boot", wait_for_boot_mode),
        ("--wait-for-load", wait_for_load_mode),
        ("--seed-clone", seed_clone_mode),
    ]
)


def run_adb_command(probe_timeout: float) -> int:
    try:
        command_timeout = timeout_setting(ADB_TIMEOUT_SETTING, DEFAULT_COMMAND_TIMEOUT)
        return run_command(sys.argv[1:], timeout=command_timeout).returncode
    except subprocess.TimeoutExpired:
        print(f"qa: ADB command timed out: {' '.join(sys.argv[1:])}", file=sys.stderr)
        return 124


def main() -> int:
    probe_timeout = timeout_setting(ADB_TIMEOUT_SETTING, DEFAULT_PROBE_TIMEOUT)
    return MODES.get(sys.argv[1], run_adb_command)(probe_timeout)


if __name__ == "__main__":
    raise SystemExit(main())
