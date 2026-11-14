import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

FALCO_DIR = Path(__file__).resolve().parent
REPO_ROOT = FALCO_DIR.parents[2]
SCRIPT = FALCO_DIR / "validate-rules.sh"

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
    return subprocess.run(
        [str(SCRIPT), *map(str, rules_files)],
        capture_output=True,
        text=True,
        check=False,
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


if __name__ == "__main__":
    unittest.main()
