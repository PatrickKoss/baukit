import importlib.util
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

REPOSITORY = Path(__file__).resolve().parents[1]
TEMPLATES = REPOSITORY / "templates/common"
SCRIPTS = (
    TEMPLATES / "__strict__/scripts/gate.py",
    TEMPLATES / "__strict__/scripts/gate.test.py",
    TEMPLATES / "__strict__/scripts/gate_disk.py",
    TEMPLATES / "__strict__/scripts/gate_disk.test.py",
    TEMPLATES / "__strict__/scripts/gate_locks.py",
    TEMPLATES / "__strict__/scripts/gate_locks.test.py",
    TEMPLATES / "__strict__/scripts/gate_results.py",
    TEMPLATES / "__strict__/scripts/gate_results.test.py",
    TEMPLATES / "scripts/process_runner.py",
)


class GateTemplateTests(unittest.TestCase):
    def test_template_gate_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in SCRIPTS:
                shutil.copy(source, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "gate.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)

    def test_template_gate_disk_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in SCRIPTS:
                shutil.copy(source, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "gate_disk.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)

    def test_template_gate_locks_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in SCRIPTS:
                shutil.copy(source, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "gate_locks.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)

    def test_template_gate_results_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in SCRIPTS:
                shutil.copy(source, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "gate_results.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)


def load_module(name):
    template_scripts = str(TEMPLATES / "__strict__/scripts")
    if template_scripts not in sys.path:
        sys.path.append(template_scripts)
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, TEMPLATES / f"__strict__/scripts/{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class BaukitCacheInputTests(unittest.TestCase):
    """Every path that --affected sends to a step is one of that step's cache inputs."""

    @classmethod
    def setUpClass(cls):
        cls.results = load_module("gate_results")
        cls.gate = load_module("gate")
        listing = run_process(
            ["sh", str(REPOSITORY / "scripts/ci-gate.sh"), "--list-steps"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        cls.steps = cls.gate.parse_steps(listing)
        cls.settings = cls.gate.load_settings(REPOSITORY / "scripts/ci-gate.toml")
        cls.affected = cls.settings["quality"]["affected"]
        cls.shared = cls.gate.shared_paths(cls.settings)
        cls.inputs = cls.results.InputMap(
            cls.affected, cls.shared, cls.gate.glob_pattern, cls.gate.base_token
        )
        cls.specs = {
            step.name: cls.results.StepSpec(step.name, step.mode, step.tokens) for step in cls.steps
        }

    def affected_steps(self, path):
        selection = self.gate.select([path], self.affected, self.shared)
        return {item.step.name for item in self.gate.plan(self.steps, selection)}

    def cache_steps(self, path):
        classes = self.results.classify([path], self.inputs)
        return {
            name for name, spec in self.specs.items() if self.results.step_inputs(spec, classes)
        }

    def test_affected_steps_always_see_the_path(self):
        paths = (
            "rust/crates/baukit-core/src/lib.rs",
            "rust/crates/baukit-sync/src/lib.rs",
            "rust/crates/baukit-auth/migrations/0001_init.sql",
            "rust/Cargo.toml",
            "cli/src/main.rs",
            "templates/common/__strict__/scripts/gate.py",
            "templates/backend/Dockerfile",
            "typescript/packages/events/src/index.ts",
            "typescript/packages/data-contracts/src/index.ts",
            "models/sync.tla",
            "docs/reliability-plan.md",
            "deploy/tools.toml",
            "examples/minimal-api/src/main.rs",
            "scripts/fuzz.sh",
            "unmatched-path.txt",
        )
        for path in paths:
            with self.subTest(path=path):
                missing = self.affected_steps(path) - self.cache_steps(path)
                self.assertEqual(missing, set())

    def test_every_tracked_path_reaches_its_affected_steps(self):
        tracked = run_process(
            ["git", "-C", str(REPOSITORY), "ls-files"], capture_output=True, text=True, check=True
        ).stdout.splitlines()
        classes = self.results.classify(tracked, self.inputs)
        inputs = {
            name: set(self.results.step_inputs(spec, classes)) for name, spec in self.specs.items()
        }
        for path in tracked:
            missing = {name for name in self.affected_steps(path) if path not in inputs[name]}
            self.assertEqual(missing, set(), path)

    def test_a_docs_change_leaves_the_heavy_steps_alone(self):
        steps = self.cache_steps("docs/reliability-plan.md")
        self.assertNotIn("rust-tests", steps)
        self.assertNotIn("platform-e2e", steps)
        self.assertIn("rust-fmt", steps)


if __name__ == "__main__":
    unittest.main()
