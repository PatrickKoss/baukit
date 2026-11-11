import importlib.util
import re
import sys
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
GATE_SCRIPT = ROOT / "scripts/ci-gate.sh"
FIXTURE_SCRIPT = ROOT / "scripts/generated-fixture-gate.sh"
GATE_PY = ROOT / "templates/common/__strict__/scripts/gate.py"
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"


def load_gate():
    sys.path.insert(0, str(ROOT / "templates/common/scripts"))
    spec = importlib.util.spec_from_file_location("template_gate", GATE_PY)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def listed_steps():
    output = run_process(
        ["sh", str(GATE_SCRIPT), "--list-steps"],
        capture_output=True,
        text=True,
        check=True,
        cwd=ROOT,
    ).stdout
    return load_gate().parse_steps(output)


def shard_flavors():
    text = GATE_SCRIPT.read_text()
    shards = dict(re.findall(r'^fixture_shard_(\d+)="([^"]+)"$', text, re.MULTILINE))
    return {number: flavors.split() for number, flavors in shards.items()}


def fixture_script_flavors():
    text = FIXTURE_SCRIPT.read_text()
    flavors = re.search(r"^flavors=\(([^)]*)\)", text, re.MULTILINE)
    return flavors.group(1).split()


class CiGateTests(unittest.TestCase):
    def test_every_step_function_exists_for_each_listed_step(self):
        functions = set(re.findall(r"^step_(\w+)\(\)", GATE_SCRIPT.read_text(), re.MULTILINE))
        for step in listed_steps():
            self.assertIn(step.name.replace("-", "_"), functions, step.name)

    def test_shards_cover_every_fixture_flavor_exactly_once(self):
        assigned = [flavor for flavors in shard_flavors().values() for flavor in flavors]
        self.assertCountEqual(assigned, fixture_script_flavors())

    def test_each_shard_has_its_own_step_and_lane(self):
        steps = {step.name: step for step in listed_steps()}
        for number in shard_flavors():
            step = steps[f"fixture-{number}"]
            self.assertEqual(step.lane, f"fixtures-{number}")
            self.assertIn("fixtures", step.tokens)
            self.assertEqual(step.after, ("ts-check",))

    def test_fixture_lanes_answer_to_the_fixtures_token(self):
        selected = {step.name for step in listed_steps() if "fixtures" in step.tokens}
        self.assertEqual(
            selected,
            {"fixture-1", "fixture-2", "affected-fixture"},
        )

    def test_standalone_fixture_steps_stay_callable(self):
        text = GATE_SCRIPT.read_text()
        self.assertIn("step_generated_fixture()", text)
        self.assertIn("step_platform_fixture()", text)

    def test_ci_matrix_covers_the_same_flavors(self):
        matrix = re.findall(r"^\s+- flavor: (\S+)$", CI_WORKFLOW.read_text(), re.MULTILINE)
        self.assertCountEqual(matrix, fixture_script_flavors())


if __name__ == "__main__":
    unittest.main()
