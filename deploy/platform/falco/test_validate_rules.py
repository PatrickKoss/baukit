import importlib.util
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

FALCO_DIR = Path(__file__).resolve().parent
REPO_ROOT = FALCO_DIR.parents[2]
SCRIPT = FALCO_DIR / "validate-rules.sh"
PLATFORM_DIR = FALCO_DIR.parent
BAUKIT_RULES = FALCO_DIR / "rules/baukit.rules.yaml"


def load_process_runner():
    spec = importlib.util.spec_from_file_location(
        "process_runner", REPO_ROOT / "scripts/process_runner.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PROCESS_RUNNER = load_process_runner()

BROKEN_RULES = {
    "undefined macro": """
- rule: Broken baukit rule
  desc: Uses a macro nobody defined.
  condition: spawned_process and no_such_baukit_macro
  output: broken | process=%proc.name
  priority: WARNING
""",
    "misspelled output field": """
- rule: Broken baukit rule
  desc: Prints a field that does not exist.
  condition: spawned_process
  output: broken | pod=%k8s.pod.nmae
  priority: WARNING
""",
}


def validate(*rules_files: Path) -> subprocess.CompletedProcess[str]:
    return PROCESS_RUNNER.run_process(
        [str(SCRIPT), *map(str, rules_files)], capture_output=True, text=True
    )


class ValidateRulesTest(unittest.TestCase):
    def test_shipped_rules_validate(self) -> None:
        result = validate()

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("baukit.rules.yaml: Ok", result.stdout)

    def test_broken_rules_fail(self) -> None:
        for name, content in BROKEN_RULES.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as directory:
                rules_file = Path(directory) / "broken.rules.yaml"
                rules_file.write_text(content, encoding="utf-8")

                result = validate(rules_file)

                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


class ImagePinTest(unittest.TestCase):
    def test_release_uses_the_validated_falco_image(self) -> None:
        manifest = tomllib.loads((REPO_ROOT / "deploy/tools.toml").read_text(encoding="utf-8"))
        image = manifest["images"]["falco"]
        tag = image.split(":", 1)[1]

        release = (FALCO_DIR / "helm-release.yaml").read_text(encoding="utf-8")

        self.assertIn(f'tag: "{tag}"', release)


def macro_condition(text: str, name: str) -> str:
    match = re.search(
        rf"^- macro: {name}\n  condition: (>\n(?:    .*\n)+|.*\n)", text, re.MULTILINE
    )
    if match is None:
        raise AssertionError(f"macro {name} not found")
    return " ".join(match.group(1).removeprefix(">").split())


def config_maps(rendered: str) -> dict[str, str]:
    found = {}
    for document in re.split(r"^---$", rendered, flags=re.MULTILINE):
        name = re.search(r"^  name: (\S+)$", document, re.MULTILINE)
        if re.search(r"^kind: ConfigMap$", document, re.MULTILINE) and name:
            found[name.group(1)] = document
    return found


def kustomize_build(directory: Path) -> str:
    tools = REPO_ROOT / ".tools/bin/kustomize"
    binary = str(tools) if os.access(tools, os.X_OK) else shutil.which("kustomize")
    if binary is None:
        raise AssertionError("kustomize missing; run scripts/install-deploy-tools.sh")
    result = PROCESS_RUNNER.run_process(
        [binary, "build", str(directory)], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return result.stdout


class SharedKernelFilterTest(unittest.TestCase):
    def test_base_watches_host_processes(self) -> None:
        rules = BAUKIT_RULES.read_text(encoding="utf-8")
        condition = macro_condition(rules, "baukit_shared_kernel")

        self.assertEqual(condition, "(never_true)")

    def test_composed_platform_keeps_the_base_default(self) -> None:
        rendered = kustomize_build(PLATFORM_DIR / "overlays/everything")

        maps = config_maps(rendered)
        base = maps["falco-baukit-rules"]
        self.assertIn("baukit_shared_kernel\n      condition: (never_true)", base)
        self.assertNotIn("falco-overlay-rules", maps)

    def test_local_component_turns_the_filter_on(self) -> None:
        rendered = kustomize_build(PLATFORM_DIR / "overlays/local/components/falco")

        overlay = config_maps(rendered)["falco-overlay-rules"]
        self.assertIn("  namespace: falco", overlay)
        self.assertRegex(
            overlay,
            r"- macro: baukit_shared_kernel\n\s+condition: \(evt\.num >= 0\)\n"
            r"\s+override:\n\s+condition: replace",
        )


class RuntimeExecTest(unittest.TestCase):
    rules = BAUKIT_RULES.read_text(encoding="utf-8")

    def test_an_exec_with_a_lost_parent_still_counts(self) -> None:
        # Upstream container_entrypoint is "not proc.pname exists or proc.pname in (runc, ...)".
        condition = macro_condition(self.rules, "baukit_runtime_exec")

        self.assertIn("container_entrypoint", condition)
        self.assertNotIn("pname", condition)

    def test_cnpg_exceptions_name_each_process(self) -> None:
        child = macro_condition(self.rules, "baukit_cnpg_instance_manager_child")
        exec_ = macro_condition(self.rules, "baukit_cnpg_runtime_exec")

        self.assertIn("k8s.ns.name = postgres", child)
        self.assertIn("container.image.repository = ghcr.io/cloudnative-pg/postgresql", child)
        self.assertEqual(
            set(re.findall(r"proc\.name = (\S+?)\)?(?:\s|$)", child)),
            {"pg_isready", "sh", "barman-cloud-wa"},
        )
        self.assertIn('proc.cmdline startswith "sh -c -- /controller/manager wal-archive "', child)
        self.assertEqual(
            set(re.findall(r'proc\.cmdline (?:=|startswith) "([^"]+)"', exec_)),
            {"manager backup ", "manager healthcheck unix"},
        )

    def test_barman_plugin_exception_names_only_the_sidecar_scripts(self) -> None:
        plugin = macro_condition(self.rules, "baukit_cnpg_plugin_child")
        exec_ = macro_condition(self.rules, "baukit_cnpg_runtime_exec")

        self.assertIn("or baukit_cnpg_plugin_child", exec_)
        self.assertIn("k8s.ns.name = postgres", plugin)
        sidecar = "ghcr.io/cloudnative-pg/plugin-barman-cloud-sidecar"
        self.assertIn(f"container.image.repository = {sidecar}", plugin)
        self.assertIn("container.name = plugin-barman-cloud", plugin)
        self.assertEqual(
            set(re.findall(r"proc\.name = (\S+)", plugin)), {"barman-cloud-ba", "barman-cloud-wa"}
        )
        self.assertEqual(
            set(re.findall(r'proc\.cmdline startswith "[^ ]+ /venv/bin/([^"]+)"', plugin)),
            {
                "barman-cloud-backup-list ",
                "barman-cloud-backup-delete ",
                "barman-cloud-wal-archive ",
            },
        )
        self.assertNotRegex(plugin, r"proc\.name (=|in) \(?sh")

    def test_flux_exception_names_only_git_in_kustomize_controller(self) -> None:
        child = macro_condition(self.rules, "baukit_flux_controller_child")

        self.assertEqual(
            set(re.findall(r"(\S+) = (\S+?)\)?(?:\s|$)", child)),
            {
                ("(k8s.ns.name", "flux-system"),
                ("container.image.repository", "ghcr.io/fluxcd/kustomize-controller"),
                ("proc.name", "git"),
            },
        )
        self.assertEqual(child.count(" and "), 2)
        self.assertNotIn(" or ", child)
        self.assertNotIn("source-controller", child)
        self.assertNotIn("sh", re.findall(r"proc\.name = (\S+?)\)?(?:\s|$)", child))

    def test_protected_exec_rule_excludes_the_flux_controller_git_children(self) -> None:
        rule = self.rules.split("- rule: Baukit exec into protected platform namespace", 1)[1]
        condition = rule.split("  output:", 1)[0]

        self.assertIn("and not baukit_flux_controller_child", condition)
        self.assertIn("baukit_runtime_exec", condition)


if __name__ == "__main__":
    unittest.main()
