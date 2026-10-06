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


def main() -> int:
    probe_timeout = timeout_setting("BAUKIT_QA_ADB_TIMEOUT_SECONDS", DEFAULT_PROBE_TIMEOUT)
    if sys.argv[1] == "--wait-for-boot":
        boot_timeout = timeout_setting("BAUKIT_QA_BOOT_TIMEOUT_SECONDS", DEFAULT_BOOT_TIMEOUT)
        return wait_for_boot(sys.argv[2], sys.argv[3], probe_timeout, boot_timeout)
    try:
        return subprocess.run(sys.argv[1:], timeout=probe_timeout, check=False).returncode
    except subprocess.TimeoutExpired:
        print(f"qa: ADB command timed out: {' '.join(sys.argv[1:])}", file=sys.stderr)
        return 124


if __name__ == "__main__":
    raise SystemExit(main())
