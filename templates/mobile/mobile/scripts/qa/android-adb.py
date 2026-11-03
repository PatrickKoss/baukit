import hashlib
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


DEFAULT_PROBE_TIMEOUT = 10
DEFAULT_BOOT_TIMEOUT = 360
DEFAULT_IDLE_TIMEOUT = 180
DEFAULT_SETTLE_TIMEOUT = 600
SETTLE_WINDOW_POLLS = 5
QUIET_CORES = 1.0
SNAPSHOT_PREFIX = "baukit-clean"
SNAPSHOT_HASH_LENGTH = 12
IDLE_STABLE_PROBES = 3
POLL_INTERVAL = 2
SYSTEMUI = "com.android.systemui"
CRITICAL_PACKAGES = {
    SYSTEMUI,
    "com.android.inputmethod.latin",
    "com.google.android.inputmethod.latin",
}


def timeout_setting(name: str, default: int) -> float:
    value = float(os.environ.get(name, default))
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name} must be a positive finite number")
    return value


def wait_for_boot(adb: str, serial: str, probe_timeout: float, boot_timeout: float) -> int:
    deadline = time.monotonic() + boot_timeout
    while (remaining := deadline - time.monotonic()) > 0:
        try:
            result = subprocess.run(
                [adb, "-s", serial, "shell", "getprop", "sys.boot_completed"],
                capture_output=True, text=True, check=False,
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
        result = subprocess.run(
            [adb, "-s", serial, "shell", *command],
            capture_output=True, text=True, check=False, timeout=timeout,
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


def settle(adb: str, serial: str, probe_timeout: float, settle_timeout: float,
           clock=time.monotonic, sleep=time.sleep) -> int:
    start = clock()
    deadline = start + settle_timeout
    shell(adb, serial, max(probe_timeout, deadline - clock()), "cmd", "package", "bg-dexopt-job")
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
        sleep(min(window, max(0, deadline - clock())))
    print(f"qa: {serial} did not settle within {settle_timeout:.0f} s; saving the snapshot anyway")
    return 0


def snapshot_name(config: Path, sdk_root: Path, gpu: str, version: str) -> str:
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
        version,
    ):
        digest.update(part.encode() + b"\0")
    return f"{SNAPSHOT_PREFIX}-{digest.hexdigest()[:SNAPSHOT_HASH_LENGTH]}"


def prune_snapshots(snapshots: Path, keep: str) -> int:
    if not snapshots.is_dir():
        return 0
    for entry in snapshots.iterdir():
        if entry.name.startswith(SNAPSHOT_PREFIX) and entry.name != keep and entry.is_dir():
            shutil.rmtree(entry)
            print(f"qa: removed stale quick-boot snapshot {entry.name}")
    return 0


def attached(adb: str, serial: str, timeout: float) -> int:
    try:
        result = subprocess.run(
            [adb, "-s", serial, "get-state"], capture_output=True, text=True, check=False, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return 124
    return 0 if result.returncode == 0 and result.stdout.strip() == "device" else 1


def wait_for_stop(adb: str, serial: str, timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while (remaining := deadline - time.monotonic()) > 0:
        result = subprocess.run(
            [adb, "devices"], capture_output=True, text=True, check=False,
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
            output.write(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {serial} status={status}\n")
            output.write("\n".join(lines) + "\n")
    return status


def anr_details(adb: str, serial: str, timeout: float) -> tuple[list[str], set[str]]:
    try:
        result = subprocess.run(
            [adb, "-s", serial, "shell", "dumpsys", "activity", "lastanr"],
            capture_output=True, text=True, check=False, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return ["qa: Android ANR diagnostics timed out"], set()
    if result.returncode != 0:
        return [f"qa: Android ANR diagnostics failed: exit {result.returncode}"], set()
    prefixes = ("ANR time:", "ANR in ", "PID:", "Reason:", "ErrorId:", "Frozen:")
    lines = [line for line in result.stdout.splitlines() if line.strip().startswith(prefixes)]
    packages = {
        package for package in CRITICAL_PACKAGES for line in lines
        if re.search(rf"(?<![\w.]){re.escape(package)}(?![\w.])", line)
    }
    return lines, packages


def check_health(adb: str, serial: str, timeout: float, report: Path | None = None) -> int:
    try:
        result = subprocess.run(
            [adb, "-s", serial, "logcat", "-d", "-b", "events", "-s", "am_anr", "am_crash"],
            capture_output=True, text=True, check=False, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return health_result(124, serial, ["qa: Android system health probe timed out"], report)
    if result.returncode != 0:
        return health_result(result.returncode, serial, ["qa: Android system health probe failed"], report)
    evidence = []
    anr_packages = set()
    for line in result.stdout.splitlines():
        fields = line.partition("[")[2].split(",")
        if len(fields) >= 3 and fields[2].strip() in CRITICAL_PACKAGES:
            package = fields[2].strip()
            evidence.extend((f"qa: Android system process is unhealthy: {package}", line))
            if "am_anr" in line:
                anr_packages.add(package)
    details, dumpsys_packages = anr_details(adb, serial, timeout)
    if anr_packages and not anr_packages & dumpsys_packages:
        evidence.append("qa: logcat am_anr shows the ANR above; dumpsys lastanr has no record of it")
    for package in sorted(dumpsys_packages - anr_packages):
        evidence.append(f"qa: dumpsys lastanr reports an ANR in {package}")
    if dumpsys_packages or (anr_packages and details[:1] and details[0].startswith("qa: ")):
        evidence.extend(details)
    return health_result(int(bool(evidence)), serial, evidence, report)


def main() -> int:
    probe_timeout = timeout_setting("BAUKIT_QA_ADB_TIMEOUT_SECONDS", DEFAULT_PROBE_TIMEOUT)
    if sys.argv[1] == "--check-health":
        report = Path(sys.argv[4]) if len(sys.argv) > 4 else None
        return check_health(sys.argv[2], sys.argv[3], probe_timeout, report)
    if sys.argv[1] == "--wait-for-idle":
        idle_timeout = timeout_setting("BAUKIT_QA_IDLE_TIMEOUT_SECONDS", DEFAULT_IDLE_TIMEOUT)
        return wait_for_idle(sys.argv[2], sys.argv[3], probe_timeout, idle_timeout)
    if sys.argv[1] == "--settle":
        settle_timeout = timeout_setting("BAUKIT_QA_SETTLE_TIMEOUT_SECONDS", DEFAULT_SETTLE_TIMEOUT)
        return settle(sys.argv[2], sys.argv[3], probe_timeout, settle_timeout)
    if sys.argv[1] == "--snapshot-name":
        print(snapshot_name(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4], sys.argv[5]))
        return 0
    if sys.argv[1] == "--prune-snapshots":
        return prune_snapshots(Path(sys.argv[2]), sys.argv[3])
    if sys.argv[1] == "--attached":
        return attached(sys.argv[2], sys.argv[3], probe_timeout)
    if sys.argv[1] == "--wait-for-boot":
        boot_timeout = timeout_setting("BAUKIT_QA_BOOT_TIMEOUT_SECONDS", DEFAULT_BOOT_TIMEOUT)
        return wait_for_boot(sys.argv[2], sys.argv[3], probe_timeout, boot_timeout)
    try:
        if sys.argv[1] == "--wait-for-stop":
            return wait_for_stop(sys.argv[2], sys.argv[3], probe_timeout)
        return subprocess.run(sys.argv[1:], timeout=probe_timeout, check=False).returncode
    except subprocess.TimeoutExpired:
        print(f"qa: ADB command timed out: {' '.join(sys.argv[1:])}", file=sys.stderr)
        return 124


if __name__ == "__main__":
    raise SystemExit(main())
