import hashlib
import math
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_PROBE_TIMEOUT = 10
DEFAULT_COMMAND_TIMEOUT = 60
ADB_TIMEOUT_SETTING = "BAUKIT_QA_ADB_TIMEOUT_SECONDS"
DEFAULT_BOOT_TIMEOUT = 360
DEFAULT_IDLE_TIMEOUT = 180
DEFAULT_SETTLE_TIMEOUT = 600
DEFAULT_WIFI_TIMEOUT = 90
HOST_ALIAS = "10.0.2.2"
WIFI_STATUS_LINES = 4
SETTLE_WINDOW_POLLS = 5
QUIET_CORES = 1.0
SNAPSHOT_PREFIX = "baukit-clean"
SNAPSHOT_HASH_LENGTH = 12
IDLE_STABLE_PROBES = 3
MIN_EVENT_FIELDS = 3
HEALTH_BASE_ARGUMENTS = 4
POLL_INTERVAL = 2
SYSTEMUI = "com.android.systemui"
CRITICAL_PACKAGES = {
    SYSTEMUI,
    "com.android.inputmethod.latin",
    "com.google.android.inputmethod.latin",
}


def run_command(command: list[str], **options) -> subprocess.CompletedProcess:
    return subprocess.run(  # noqa: S603 -- adb invocations or the caller's own argv
        command, check=False, **options
    )


def timeout_setting(name: str, default: int) -> float:
    value = float(os.environ.get(name, default))
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name} must be a positive finite number")
    return value


def wait_for_boot(adb: str, serial: str, probe_timeout: float, boot_timeout: float) -> int:
    deadline = time.monotonic() + boot_timeout
    while (remaining := deadline - time.monotonic()) > 0:
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
        time.sleep(min(POLL_INTERVAL, max(0, deadline - time.monotonic())))
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


def systemui_idle_probe(adb: str, serial: str, timeout: float) -> str | None:
    """Return the systemui PID when systemui runs and the focused window is no ANR dialog."""
    pid = (shell(adb, serial, timeout, "pidof", SYSTEMUI) or "").strip()
    windows = shell(adb, serial, timeout, "dumpsys", "window", "displays") or ""
    focus = [line for line in windows.splitlines() if "mCurrentFocus=" in line]
    if not pid or not focus or "null" in focus[0] or "Not Responding" in focus[0]:
        return None
    return pid


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


def wait_for_idle(adb: str, serial: str, probe_timeout: float, idle_timeout: float) -> int:
    deadline = time.monotonic() + idle_timeout
    stable = 0
    last_pid = None
    while deadline - time.monotonic() > 0:
        pid = systemui_idle_probe(adb, serial, probe_timeout)
        stable = stable + 1 if pid is not None and pid == last_pid else 0
        last_pid = pid
        if stable >= IDLE_STABLE_PROBES - 1:
            return 0
        time.sleep(min(poll_interval(), max(0, deadline - time.monotonic())))
    print("qa: systemui did not settle before the idle deadline", file=sys.stderr)
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
) -> int:
    start = time.monotonic()
    deadline = start + settle_timeout
    shell(
        adb,
        serial,
        max(probe_timeout, deadline - time.monotonic()),
        "cmd",
        "package",
        "bg-dexopt-job",
    )
    window = SETTLE_WINDOW_POLLS * poll_interval()
    before = None
    while time.monotonic() < deadline:
        stat = shell(adb, serial, probe_timeout, "cat", "/proc/stat")
        after = cpu_counters(stat) if stat else None
        if after is None:
            if attached(adb, serial, probe_timeout):
                print(f"qa: lost {serial} while it settled", file=sys.stderr)
                return 1
        elif before is not None and busy_cores(before, after) < QUIET_CORES:
            print(f"qa: {serial} settled in {time.monotonic() - start:.0f} s")
            return 0
        before = after
        time.sleep(min(window, max(0, deadline - time.monotonic())))
    print(f"qa: {serial} did not settle within {settle_timeout:.0f} s; saving the snapshot anyway")
    return 0


def snapshot_name(config: Path, sdk_root: Path, gpu: str, features: str, version: str) -> str:
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
        version,
    ):
        digest.update(part.encode() + b"\0")
    return f"{SNAPSHOT_PREFIX}-{digest.hexdigest()[:SNAPSHOT_HASH_LENGTH]}"


def loaded_snapshots(avd_name: str, proc: Path = Path("/proc")) -> set[str]:
    loaded = set()
    for cmdline in proc.glob("[0-9]*/cmdline"):
        try:
            args = cmdline.read_bytes().decode(errors="replace").split("\0")
        except OSError:
            continue
        pairs = dict(zip(args, args[1:], strict=False))
        if pairs.get("-avd") == avd_name and "-snapshot" in pairs:
            loaded.add(pairs["-snapshot"])
    return loaded


def prune_snapshots(avd_dir: Path, keep: str, loaded: set[str]) -> int:
    """Delete older baukit snapshots except loaded ones and the newest complete older one."""
    snapshots = avd_dir / "snapshots"
    older = [
        entry
        for entry in (snapshots.iterdir() if snapshots.is_dir() else ())
        if entry.name.startswith(SNAPSHOT_PREFIX) and entry.name != keep and entry.is_dir()
    ]
    complete = [entry for entry in older if (avd_dir / f"{entry.name}.ready").is_file()]
    kept = {keep, *loaded}
    if complete:
        kept.add(max(complete, key=lambda entry: entry.stat().st_mtime).name)
    for entry in older:
        if entry.name in kept:
            print(f"qa: kept older quick-boot snapshot {entry.name} for other worktrees")
            continue
        shutil.rmtree(entry)
        print(f"qa: removed stale quick-boot snapshot {entry.name}")
    for ready in avd_dir.glob(f"{SNAPSHOT_PREFIX}*.ready"):
        if ready.name.removesuffix(".ready") not in kept:
            ready.unlink()
    return 0


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


def wait_for_stop(adb: str, serial: str, timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while (remaining := deadline - time.monotonic()) > 0:
        result = run_command(
            [adb, "devices"],
            capture_output=True,
            text=True,
            timeout=remaining,
        )
        if result.returncode != 0:
            print("qa: Android shutdown probe failed", file=sys.stderr)
            return result.returncode
        if not any(line.split()[:1] == [serial] for line in result.stdout.splitlines()):
            return 0
        time.sleep(min(POLL_INTERVAL, max(0, deadline - time.monotonic())))
    print(f"qa: {serial} did not stop before the ADB deadline", file=sys.stderr)
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


def snapshot_name_mode(_probe_timeout: float) -> int:
    print(
        snapshot_name(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4], sys.argv[5], sys.argv[6])
    )
    return 0


def prune_snapshots_mode(_probe_timeout: float) -> int:
    return prune_snapshots(Path(sys.argv[2]), sys.argv[3], loaded_snapshots(sys.argv[4]))


def attached_mode(probe_timeout: float) -> int:
    return attached(sys.argv[2], sys.argv[3], probe_timeout)


MODES = dict(
    [
        ("--check-health", check_health_mode),
        timed_mode(
            "--wait-for-idle", "BAUKIT_QA_IDLE_TIMEOUT_SECONDS", DEFAULT_IDLE_TIMEOUT, wait_for_idle
        ),
        timed_mode("--settle", "BAUKIT_QA_SETTLE_TIMEOUT_SECONDS", DEFAULT_SETTLE_TIMEOUT, settle),
        ("--snapshot-name", snapshot_name_mode),
        ("--prune-snapshots", prune_snapshots_mode),
        timed_mode(
            "--wait-for-wifi-route",
            "BAUKIT_QA_WIFI_TIMEOUT_SECONDS",
            DEFAULT_WIFI_TIMEOUT,
            wait_for_wifi_route,
        ),
        ("--attached", attached_mode),
        timed_mode(
            "--wait-for-boot", "BAUKIT_QA_BOOT_TIMEOUT_SECONDS", DEFAULT_BOOT_TIMEOUT, wait_for_boot
        ),
    ]
)


def run_adb_command(probe_timeout: float) -> int:
    try:
        if sys.argv[1] == "--wait-for-stop":
            return wait_for_stop(sys.argv[2], sys.argv[3], probe_timeout)
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
