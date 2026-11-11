from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

from process_runner import run_process

SCRIPT = Path(__file__).resolve().parent / "with-loopback-network.sh"


def run(*command: str) -> subprocess.CompletedProcess[str]:
    return run_process([str(SCRIPT), *command], capture_output=True, text=True, check=False)


class WithLoopbackNetworkTest(unittest.TestCase):
    def test_command_sees_only_an_active_loopback_interface(self) -> None:
        result = run("ip", "-o", "link", "show")
        self.assertEqual(result.returncode, 0, result.stderr)
        links = [line.split(":")[1].strip() for line in result.stdout.splitlines()]
        self.assertEqual(links, ["lo"])
        self.assertIn("UP", result.stdout)

    def test_command_exit_status_and_arguments_pass_through(self) -> None:
        result = run("sh", "-c", 'printf "%s|" "$@"; exit 7', "probe", "a b", "c")
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, "a b|c|")

    def test_usage_without_a_command(self) -> None:
        result = run()
        self.assertEqual(result.returncode, 64)
        self.assertIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
