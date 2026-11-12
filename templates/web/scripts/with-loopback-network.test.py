import os
import stat
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

SCRIPT = Path(__file__).resolve().parent / "with-loopback-network.sh"
UNSHARE_UNAVAILABLE = "#!/bin/sh\nexit 1\n"


def run(*command, env=None):
    return run_process(
        ["sh", str(SCRIPT), *command],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


class WithLoopbackNetworkTest(unittest.TestCase):
    def test_command_sees_only_an_active_loopback_interface(self):
        result = run("ip", "-o", "link", "show")
        self.assertEqual(result.returncode, 0, result.stderr)
        links = [line.split(":")[1].strip() for line in result.stdout.splitlines()]
        self.assertEqual(links, ["lo"])
        self.assertIn("UP", result.stdout)

    def test_exit_status_and_arguments_pass_through(self):
        result = run("sh", "-c", 'printf "%s|" "$@"; exit 7', "probe", "a b", "c")
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, "a b|c|")

    def test_usage_without_a_command(self):
        result = run()
        self.assertEqual(result.returncode, 64)
        self.assertIn("usage:", result.stderr)

    def test_missing_user_namespaces_exit_69_without_running_the_command(self):
        with tempfile.TemporaryDirectory() as directory:
            shim = Path(directory) / "unshare"
            shim.write_text(UNSHARE_UNAVAILABLE)
            shim.chmod(shim.stat().st_mode | stat.S_IXUSR)
            marker = Path(directory) / "ran"
            env = {**os.environ, "PATH": f"{directory}:{os.environ['PATH']}"}
            result = run("touch", str(marker), env=env)
            self.assertEqual(result.returncode, 69, result.stderr)
            self.assertIn("unavailable", result.stderr)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
