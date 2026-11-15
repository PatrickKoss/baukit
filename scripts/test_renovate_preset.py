"""Check the Renovate custom managers against the files they target.

Renovate evaluates matchStrings with RE2 or JavaScript RegExp. The patterns use only syntax that
behaves the same in Python `re` once `(?<name>` becomes `(?P<name>`: no inline flags, lookbehind or
backreferences. Validate the schema with
`npx --yes --package renovate@44.198.3 -- renovate-config-validator --strict` in a copy named
renovate.json.
"""

from __future__ import annotations

import json
import re
import tomllib
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
PRESET = json.loads((ROOT / "renovate-preset.json").read_text())
PLATFORM = ROOT / "deploy/platform"
PYTHON_GROUP = re.compile(r"\(\?<(?=[A-Za-z])")
OCI_HELM_LAYER = "mediaType: application/vnd.cncf.helm.chart.content.v1.tar+gzip"
OCI_REF = re.compile(r'\n  ref:\n    tag: "[^"]+"\n    digest: sha256:[0-9a-f]{64}\n')
UNSUPPORTED = re.compile(r"\(\?[a-z]+\)|\(\?<[=!]|\\[1-9]")


def python_pattern(pattern: str) -> re.Pattern[str]:
    return re.compile(PYTHON_GROUP.sub("(?P<", pattern))


def file_pattern(pattern: str) -> re.Pattern[str]:
    return re.compile(pattern.strip("/"))


def tracked_files() -> list[str]:
    output = run_process(
        ["git", "ls-files"], capture_output=True, text=True, check=True, cwd=ROOT
    ).stdout
    return output.splitlines()


def manager(description_prefix: str) -> dict:
    found = [m for m in PRESET["customManagers"] if m["description"].startswith(description_prefix)]
    if len(found) != 1:
        raise AssertionError(f"expected one manager for {description_prefix!r}, got {len(found)}")
    return found[0]


def matches(custom: dict) -> list[tuple[str, dict[str, str]]]:
    targets = [file_pattern(pattern) for pattern in custom["managerFilePatterns"]]
    results = []
    for name in tracked_files():
        if not any(target.search(name) for target in targets):
            continue
        text = (ROOT / name).read_text()
        for pattern in custom["matchStrings"]:
            for match in python_pattern(pattern).finditer(text):
                groups = {
                    key: value for key, value in match.groupdict().items() if value is not None
                }
                results.append((name, groups))
    return results


class RenovatePresetTests(unittest.TestCase):
    def test_patterns_avoid_syntax_that_differs_between_engines(self):
        for custom in PRESET["customManagers"]:
            for pattern in custom["matchStrings"]:
                with self.subTest(pattern=pattern):
                    self.assertIsNone(UNSUPPORTED.search(pattern))

    def test_every_tool_version_has_exactly_one_manager(self):
        tools = tomllib.loads((ROOT / "deploy/tools.toml").read_text())["tools"]
        found: list[tuple[str, str]] = []
        for custom in PRESET["customManagers"]:
            for name, groups in matches(custom):
                if name == "deploy/tools.toml" and "currentDigest" not in groups:
                    found.append((groups.get("depName", "yamllint"), groups["currentValue"]))
        self.assertCountEqual(found, [(name, tool["version"]) for name, tool in tools.items()])

    def test_github_tools_resolve_to_their_repository(self):
        packages = {
            groups["depName"]: groups["packageName"]
            for _, groups in matches(manager("GitHub release tools"))
        }
        self.assertEqual(packages["kustomize"], "kubernetes-sigs/kustomize")
        self.assertEqual(packages["trivy"], "aquasecurity/trivy")
        self.assertNotIn("helm", packages)

    def test_kustomize_release_tags_strip_their_prefix(self):
        template = manager("GitHub release tools")["extractVersionTemplate"]
        for tag in ["kustomize/v5.8.3", "v0.8.0", "0.2.15"]:
            with self.subTest(tag=tag):
                self.assertRegex(python_pattern(template).match(tag).group("version"), r"^\d")
        self.assertIsNone(python_pattern(template).match("api/v0.20.1"))

    def test_validator_images_keep_their_digest(self):
        images = {groups["depName"]: groups for _, groups in matches(manager("Validator images"))}
        tools = tomllib.loads((ROOT / "deploy/tools.toml").read_text())["images"]
        self.assertEqual(len(images), len(tools))
        for groups in images.values():
            self.assertRegex(groups["currentDigest"], r"^sha256:[a-f0-9]{64}$")

    def test_helm_values_digests_are_found(self):
        found = {
            (groups.get("registry"), groups["repository"])
            for _, groups in matches(manager("Helm values"))
        }
        self.assertIn(("docker.io", "falcosecurity/falco"), found)
        self.assertIn(("docker.io", "falcosecurity/falcosidekick"), found)

    def test_helm_values_digest_key_shape_is_found(self):
        text = (
            "    image:\n      repository: ghcr.io/example/app\n"
            + '      tag: "1.2.3"\n      digest: sha256:'
            + "b" * 64
        )
        pattern = python_pattern(manager("Helm values")["matchStrings"][1])
        groups = pattern.search(text).groupdict()
        self.assertEqual(
            (groups["repository"], groups["currentValue"]), ("ghcr.io/example/app", "1.2.3")
        )

    def test_k3s_strings_are_found_in_patches_scripts_and_docs(self):
        found = {name for name, _ in matches(manager("Kustomize patches"))}
        found |= {name for name, _ in matches(manager("k3s image"))}
        tracked = [
            name
            for name in tracked_files()
            if name.startswith("deploy/platform/")
            and name.endswith((".yaml", ".sh", ".md"))
            and "rancher/k3s:" in (ROOT / name).read_text()
        ]
        kubernetes_manifests = {
            name
            for name in tracked
            if re.search(r"^\s+image: rancher/k3s:", (ROOT / name).read_text(), re.M)
        }
        self.assertEqual(found, set(tracked) - kubernetes_manifests)

    def test_every_platform_directory_has_its_own_group(self):
        directories = {
            path.name
            for path in PLATFORM.iterdir()
            if path.is_dir() and (path / "kustomization.yaml").is_file()
        }
        directories |= {"overlays", "components"}
        grouped = {
            rule["matchFileNames"][0].removeprefix("deploy/platform/").removesuffix("/**")
            for rule in PRESET["packageRules"]
            if (rule.get("groupName") or "").startswith("platform ")
        }
        self.assertEqual(grouped, directories)

    def test_base_groups_override_the_shared_group_and_majors_stay_isolated(self):
        rules = PRESET["packageRules"]
        names = [rule.get("groupName") for rule in rules]
        self.assertLess(names.index("non-major dependencies"), names.index("platform falco"))
        self.assertEqual(rules[-1]["matchUpdateTypes"], ["major"])
        self.assertIsNone(rules[-1]["groupName"])

    def test_helm_charts_wait_before_they_update(self):
        rule = next(
            rule for rule in PRESET["packageRules"] if rule.get("matchDatasources") == ["helm"]
        )
        self.assertEqual(rule["minimumReleaseAge"], "7 days")

    def test_oci_helm_charts_wait_before_they_update(self):
        rule = next(
            rule for rule in PRESET["packageRules"] if rule.get("matchDepTypes") == ["HelmChart"]
        )
        self.assertEqual(rule["matchManagers"], ["flux"])
        self.assertEqual(rule["minimumReleaseAge"], "7 days")

    def test_platform_oci_charts_are_shaped_for_the_flux_manager(self):
        sources = [
            name
            for name in tracked_files()
            if name.startswith("deploy/platform/")
            and "/lint/" not in name
            and "/tests/" not in name
            and name.endswith(".yaml")
            and "\nkind: OCIRepository\n" in (ROOT / name).read_text()
        ]
        self.assertGreaterEqual(len(sources), 10)
        for name in sources:
            for document in (ROOT / name).read_text().split("---\n"):
                if "\nkind: OCIRepository\n" not in document:
                    continue
                self.assertIn(OCI_HELM_LAYER, document, name)
                self.assertRegex(document, OCI_REF, name)


if __name__ == "__main__":
    unittest.main()
