import importlib.util
import re
import sys
import tomllib
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
    sys.path.insert(0, str(GATE_PY.parent))
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


def step_body(name):
    pattern = rf"^step_{name}\(\) \{{\n(.*?)^\}}"
    match = re.search(pattern, GATE_SCRIPT.read_text(), re.MULTILINE | re.DOTALL)
    return match.group(1) if match else ""


def required_tools(script):
    text = (ROOT / script).read_text()
    match = re.search(r'^baukit_use_tools "\$repo_root" (.+)$', text, re.MULTILINE)
    return set(match.group(1).split()) if match else set()


class CiGateTests(unittest.TestCase):
    def test_deploy_steps_install_the_pinned_tools_their_scripts_use(self):
        cases = {
            "platform_validate": "deploy/platform/validate.sh",
            "observability_rules": "deploy/observability/lint/check-rules.sh",
            "platform_images": "scripts/platform-images.sh",
        }
        for step, script in cases.items():
            with self.subTest(step=step):
                body = step_body(step)
                install = re.search(r"scripts/install-deploy-tools\.sh --only (\S+)", body)
                self.assertIsNotNone(install, body)
                self.assertLess(install.start(), body.index(script.removeprefix("./")))
                tools = required_tools(script)
                self.assertTrue(tools, script)
                self.assertEqual(set(install.group(1).split(",")), tools)

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

    def test_platform_e2e_is_its_own_selected_lane(self):
        step = {step.name: step for step in listed_steps()}["platform-e2e"]
        self.assertEqual((step.lane, step.after, step.mode), ("platform-e2e", (), "selected"))
        text = GATE_SCRIPT.read_text()
        self.assertIn("step_platform_e2e() {\n", text)
        self.assertIn("--only chainsaw,flux,k3d,kubectl\n  ./deploy/platform/e2e/run.sh\n}", text)

    def test_platform_e2e_runs_for_deploy_changes_but_not_for_rust_changes(self):
        gate = load_gate()
        settings = tomllib.loads((ROOT / "scripts/ci-gate.toml").read_text())
        affected = settings["quality"]["affected"]
        shared = gate.shared_paths(settings)

        def planned(path):
            selection = gate.select([path], affected, shared)
            return {item.step.name for item in gate.plan(listed_steps(), selection)}

        self.assertIn("platform-e2e", planned("deploy/platform/kyverno/policies.yaml"))
        self.assertIn("platform-e2e", planned("deploy/chart/baukit-app/values.yaml"))
        self.assertNotIn("platform-e2e", planned("rust/crates/baukit-core/src/lib.rs"))

    def test_ci_matrix_covers_the_same_flavors(self):
        matrix = re.findall(r"^\s+- flavor: (\S+)$", CI_WORKFLOW.read_text(), re.MULTILINE)
        self.assertCountEqual(matrix, fixture_script_flavors())


if __name__ == "__main__":
    unittest.main()
