from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "expo-android-conformance.sh"

DRIVER = r'''
set -euo pipefail
source "$1"
expo_android_init "$2" dev.baukit.conformance
corepack() {
  printf '%s\n' "$*" >> "$EVENTS"
  if [[ -n "${FAIL_COMMAND:-}" && "$*" == *"$FAIL_COMMAND"* ]]; then return 7; fi
  if [[ "$*" == *"exec expo prebuild"* ]]; then
    mkdir -p "$example_dir/android"
    touch "$example_dir/android/gradlew"
    chmod +x "$example_dir/android/gradlew"
  fi
}
adb() { printf 'adb %s\n' "$*" >> "$EVENTS"; }
expo_android_prepare @baukit/data-contracts
echo device-phase >> "$EVENTS"
'''


class ExpoAndroidPhaseTest(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.example = self.root / "examples/conformance"
        self.example.mkdir(parents=True)
        self.events = self.root / "events"
        setup = self.root / "scripts/android-sdk-setup.sh"
        setup.parent.mkdir()
        setup.write_text('#!/bin/sh\necho sdk-setup >> "$EVENTS"\n', encoding="utf-8")
        setup.chmod(0o755)

    def run_phase(
        self, phase: str | None = None, fail_command: str = ""
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment.pop("BAUKIT_ANDROID_PHASE", None)
        environment.update(EVENTS=str(self.events), FAIL_COMMAND=fail_command)
        if phase is not None:
            environment["BAUKIT_ANDROID_PHASE"] = phase
        return subprocess.run(
            ["bash", "-c", DRIVER, "test", str(SCRIPT), str(self.example)],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def read_events(self) -> list[str]:
        return self.events.read_text().splitlines() if self.events.exists() else []

    def preparation_events(self) -> list[str]:
        return [
            "sdk-setup",
            f"pnpm --dir {self.root}/typescript install --frozen-lockfile",
            f"pnpm --dir {self.root}/typescript --filter @baukit/data-contracts run build",
            f"pnpm --dir {self.example} install --frozen-lockfile",
            f"pnpm --dir {self.example} run typecheck",
            f"pnpm --dir {self.example} exec expo prebuild --clean --platform android",
        ]

    def test_default_phase_prepares_then_continues_to_device_checks(self) -> None:
        result = self.run_phase()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read_events(), self.preparation_events() + ["device-phase"])

    def test_prepare_phase_creates_wrapper_without_running_device_checks(self) -> None:
        result = self.run_phase("prepare")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(os.access(self.example / "android/gradlew", os.X_OK))
        self.assertEqual(self.read_events(), self.preparation_events())

        result = self.run_phase("run")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read_events(), self.preparation_events() + ["device-phase"])

    def test_run_phase_rejects_a_missing_prebuild(self) -> None:
        result = self.run_phase("run")

        self.assertEqual(result.returncode, 1)
        self.assertIn("Prepare the Android project", result.stderr)
        self.assertEqual(self.read_events(), [])

    def test_invalid_phase_fails_before_preparation_or_device_checks(self) -> None:
        result = self.run_phase("invalid")

        self.assertEqual(result.returncode, 2)
        self.assertIn("BAUKIT_ANDROID_PHASE must be all, prepare, or run", result.stderr)
        self.assertEqual(self.read_events(), [])

    def test_preparation_failures_stop_both_all_and_prepare_phases(self) -> None:
        for phase in ("all", "prepare"):
            for command in ("install --frozen-lockfile", "exec expo prebuild"):
                with self.subTest(phase=phase, command=command):
                    self.events.unlink(missing_ok=True)
                    result = self.run_phase(phase, command)

                    self.assertEqual(result.returncode, 7)
                    self.assertNotIn("device-phase", self.read_events())
                    self.assertFalse(
                        any(event.startswith("adb ") for event in self.read_events())
                    )


if __name__ == "__main__":
    unittest.main()
