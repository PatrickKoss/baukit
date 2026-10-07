import math
import os
import subprocess
import sys
import time


DEFAULT_PROBE_TIMEOUT = 10
DEFAULT_BOOT_TIMEOUT = 360
POLL_INTERVAL = 2


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


def check_health(adb: str, serial: str, timeout: float) -> int:
    try:
        result = subprocess.run(
            [adb, "-s", serial, "logcat", "-d", "-b", "events", "-s", "am_anr", "am_crash"],
            capture_output=True, text=True, check=False, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        print("qa: Android system health probe timed out", file=sys.stderr)
        return 124
    if result.returncode != 0:
        print("qa: Android system health probe failed", file=sys.stderr)
        return result.returncode
    critical_packages = {
        "com.android.systemui",
        "com.android.inputmethod.latin",
        "com.google.android.inputmethod.latin",
    }
    for line in result.stdout.splitlines():
        fields = line.partition("[")[2].split(",")
        if len(fields) >= 3 and fields[2].strip() in critical_packages:
            print(f"qa: Android system process is unhealthy: {fields[2].strip()}", file=sys.stderr)
            return 1
    return 0


def main() -> int:
    probe_timeout = timeout_setting("BAUKIT_QA_ADB_TIMEOUT_SECONDS", DEFAULT_PROBE_TIMEOUT)
    if sys.argv[1] == "--check-health":
        return check_health(sys.argv[2], sys.argv[3], probe_timeout)
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
