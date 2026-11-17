import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("gate_disk", SCRIPTS / "gate_disk.py")
disk = importlib.util.module_from_spec(spec)
sys.modules["gate_disk"] = disk
spec.loader.exec_module(disk)

GB = disk.BYTES_PER_GB
SETTINGS = {"quality": {"disk": {"floor_gb": 30, "step_floor_gb": {"platform-e2e": 120}}}}


def fake_command(docker_root=None, reclaimable=None, sizes=None):
    calls = []

    def command(arguments):
        calls.append(arguments)
        if arguments[:2] == ["docker", "info"]:
            return docker_root
        if arguments[:3] == ["docker", "system", "df"]:
            return reclaimable
        if arguments[0] == "du":
            return f"{(sizes or {}).get(Path(arguments[-1]).parent.name, 0)}\t{arguments[-1]}"
        return None

    command.calls = calls
    return command


class FloorTests(unittest.TestCase):
    def test_the_highest_planned_step_floor_wins(self):
        self.assertEqual(disk.required_floor_gb(SETTINGS, ["rust-fmt", "platform-e2e"], {}), 120)

    def test_steps_without_a_floor_use_the_base_floor(self):
        self.assertEqual(disk.required_floor_gb(SETTINGS, ["rust-fmt"], {}), 30)

    def test_without_settings_the_default_applies(self):
        self.assertEqual(disk.required_floor_gb({}, ["x"], {}), disk.DEFAULT_FLOOR_GB)

    def test_the_environment_overrides_the_settings(self):
        environment = {"BAUKIT_GATE_DISK_FLOOR_GB": "7"}
        self.assertEqual(disk.required_floor_gb(SETTINGS, ["platform-e2e"], environment), 7)


class DiskErrorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name) / "repo"
        for name in ("rust", "cli"):
            (self.root / name / "target").mkdir(parents=True)

    def tearDown(self):
        self.directory.cleanup()

    def test_enough_space_passes(self):
        error = disk.disk_error(
            SETTINGS,
            ["platform-e2e"],
            self.root,
            {},
            disk.Probe(fake_command(), lambda _: 121 * GB),
        )
        self.assertIsNone(error)

    def test_short_space_names_free_space_floor_and_hints(self):
        command = fake_command(
            reclaimable="Images: 80GB (60%)\nBuild Cache: 12GB",
            sizes={"rust": 30 * GB, "cli": 5 * GB},
        )
        error = disk.disk_error(
            SETTINGS, ["platform-e2e"], self.root, {}, disk.Probe(command, lambda _: 90 * GB)
        )
        self.assertIn(f"{self.root} has 90.0 GB free, below the 120 GB floor", error)
        self.assertIn("nothing was deleted", error)
        rust_line = error.index(f"30.0 GB {self.root / 'rust/target'}")
        self.assertLess(rust_line, error.index(f"5.0 GB {self.root / 'cli/target'}"))
        self.assertIn("  Images: 80GB (60%)", error)
        destructive = [call for call in command.calls if "prune" in call or "rm" in call]
        self.assertEqual(destructive, [])

    def test_a_separate_docker_filesystem_is_checked(self):
        docker = Path(self.directory.name) / "docker"
        docker.mkdir()
        paths = disk.checked_paths(self.root, docker)
        self.assertEqual(paths, [self.root])
        mounts = disk.short_mounts(
            [self.root, docker], 30, {self.root: 50 * GB, docker: 10 * GB}.get
        )
        self.assertEqual([mount.path for mount in mounts], [docker])

    def test_docker_root_on_the_same_filesystem_is_measured_once(self):
        command = fake_command(docker_root=str(self.root))
        measured = []
        disk.disk_error(
            SETTINGS,
            [],
            self.root,
            {},
            disk.Probe(command, lambda p: measured.append(p) or 99 * GB),
        )
        self.assertEqual(measured, [self.root])

    def test_a_zero_floor_skips_the_check(self):
        command = fake_command()
        error = disk.disk_error(
            SETTINGS,
            [],
            self.root,
            {"BAUKIT_GATE_DISK_FLOOR_GB": "0"},
            disk.Probe(command, lambda _: 0),
        )
        self.assertIsNone(error)
        self.assertEqual(command.calls, [])


class GateRunTests(unittest.TestCase):
    def test_the_gate_refuses_to_start_below_the_floor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "steps.sh"
            script.write_text(
                "if [ \"$1\" = --list-steps ]; then echo 'unit fast - always -'; exit 0; fi\n"
                f"touch {root}/ran\n"
            )
            settings = root / "settings.toml"
            settings.write_text("[quality.disk]\nfloor_gb = 100000000\n")
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "gate.py"),
                    "run",
                    "--script",
                    f"sh {script}",
                    "--settings",
                    str(settings),
                    "--log-dir",
                    str(root / "logs"),
                ],
                capture_output=True,
                text=True,
                check=False,
                cwd=root,
                env={
                    "PATH": "/usr/bin:/bin",
                    "PYTHONPATH": os.environ.get("PYTHONPATH", ""),
                    "BAUKIT_GATE_CORES": "2",
                },
            )
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("below the 100000000 GB floor", result.stderr)
            self.assertFalse((root / "ran").exists())


if __name__ == "__main__":
    unittest.main()
