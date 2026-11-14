from __future__ import annotations

import hashlib
import io
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
from pathlib import Path

from process_runner import run_process

REPO_ROOT = Path(__file__).resolve().parents[1]
INSTALLER = REPO_ROOT / "scripts" / "install-deploy-tools.sh"
HELPER = REPO_ROOT / "deploy" / "lib" / "tools.sh"
MANIFEST = REPO_ROOT / "deploy" / "tools.toml"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
PLATFORM = "linux-amd64"
REQUIRED_TOOLS = {
    "kubeconform",
    "kustomize",
    "helm",
    "kyverno",
    "chainsaw",
    "promtool",
    "pint",
    "trivy",
    "k3d",
    "flux",
    "yamllint",
    "kubectl",
}
REQUIRED_PLATFORMS = {"linux-amd64", "linux-arm64", "darwin-arm64"}
SHA256 = re.compile(r"[0-9a-f]{64}")
VERSION_TOKEN = r"v?(?P<version>[0-9][0-9A-Za-z.+-]*)"


def fake_tool(version: str) -> bytes:
    return f"#!/bin/sh\necho fake {version}\n".encode()


def raw_tool(name: str, version: str, url: str, digest: str) -> str:
    return (
        f'[tools.{name}]\nversion = "{version}"\nurl = "{url}"\n'
        f'sha256 = {{ {PLATFORM} = "{digest}" }}\n'
    )


def tarball(member: str, content: bytes) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        info = tarfile.TarInfo(member)
        info.size = len(content)
        info.mode = 0o755
        archive.addfile(info, io.BytesIO(content))
    return buffer.getvalue()


def workflow_version_conflicts(manifest: dict, workflows: dict[str, str]) -> list[str]:
    conflicts = []
    for tool_name, tool in manifest["tools"].items():
        for pattern in tool_version_patterns(tool_name, tool):
            for workflow, text in workflows.items():
                for match in pattern.finditer(text):
                    if match.group("version") != tool["version"]:
                        conflicts.append(
                            f"{workflow}: {tool_name} {match.group('version')} != {tool['version']}"
                        )
    return conflicts


def tool_version_patterns(tool_name: str, tool: dict) -> list[re.Pattern[str]]:
    key = re.escape(tool_name).replace("_", "[-_]")
    patterns = [
        re.compile(rf"(?i)\b{key}[-_]version\b['\"]?\s*[:=]\s*['\"]?{VERSION_TOKEN}"),
        re.compile(
            rf"(?i)setup-{key}@\S+.*(?:\n\s+(?!-\s).*){{0,6}}?\n\s+version:\s*['\"]?{VERSION_TOKEN}"
        ),
    ]
    if "url" in tool:
        prefix, _, _ = tool["url"].partition("{version}")
        patterns.append(
            re.compile(re.escape(prefix) + r"(?P<version>[0-9][0-9A-Za-z.+-]*?)(?=[/_.-])")
        )
    if "requirements" in tool:
        patterns.append(re.compile(rf"(?i)\b{key}==(?P<version>[0-9][0-9A-Za-z.+-]*)"))
    return patterns


class InstallerFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        self.tools_dir = self.root / "tools"
        self.manifest = self.root / "tools.toml"

    def publish(self, name: str, data: bytes) -> tuple[str, str]:
        path = self.downloads / name
        path.write_bytes(data)
        return path.as_uri(), hashlib.sha256(data).hexdigest()

    def write_manifest(self, body: str) -> None:
        self.manifest.write_text(
            f'schema = 1\nplatforms = ["{PLATFORM}"]\n\n{body}', encoding="utf-8"
        )

    def archive_tool(self, name: str, version: str, sha256: str | None = None) -> None:
        url, digest = self.publish(
            f"{name}-{version}.tar.gz", tarball(f"{name}-dir/{name}", fake_tool(version))
        )
        self.write_manifest(
            f"[tools.{name}]\n"
            f'version = "{version}"\n'
            f'url = "{url}"\n'
            f'member = "{name}-dir/{name}"\n'
            f'sha256 = {{ {PLATFORM} = "{sha256 or digest}" }}\n'
        )

    def run_installer(
        self, *args: str, platform: str = PLATFORM
    ) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "BAUKIT_TOOLS_MANIFEST": str(self.manifest),
            "BAUKIT_TOOLS_DIR": str(self.tools_dir),
            "BAUKIT_TOOLS_PLATFORM": platform,
        }
        return run_process(
            ["bash", str(INSTALLER), *args], capture_output=True, text=True, env=env, check=False
        )

    def installed_output(self, name: str) -> str:
        return run_process(
            [str(self.tools_dir / "bin" / name)], capture_output=True, text=True, check=True
        ).stdout


class InstallDeployToolsTest(InstallerFixture):
    def test_installs_archive_member_and_writes_marker(self) -> None:
        self.archive_tool("demo", "1.2.3")

        result = self.run_installer()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("installed demo 1.2.3", result.stdout)
        self.assertEqual(self.installed_output("demo"), "fake 1.2.3\n")
        self.assertTrue((self.tools_dir / "markers" / "demo").read_text().startswith("1.2.3 "))

    def test_installs_bare_binary(self) -> None:
        url, digest = self.publish("raw", fake_tool("9.9.9"))
        self.write_manifest(raw_tool("raw", "9.9.9", url, digest))

        result = self.run_installer()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed_output("raw"), "fake 9.9.9\n")

    def test_checksum_mismatch_fails_without_installing(self) -> None:
        self.archive_tool("demo", "1.2.3", sha256="0" * 64)

        result = self.run_installer()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch for demo", result.stderr)
        self.assertFalse((self.tools_dir / "bin" / "demo").exists())
        self.assertFalse((self.tools_dir / "markers" / "demo").exists())

    def test_unknown_platform_fails(self) -> None:
        self.archive_tool("demo", "1.2.3")

        result = self.run_installer(platform="plan9-mips")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported platform plan9-mips", result.stderr)
        self.assertFalse((self.tools_dir / "bin" / "demo").exists())

    def test_platform_without_checksum_fails(self) -> None:
        self.archive_tool("demo", "1.2.3")
        self.manifest.write_text(
            self.manifest.read_text().replace(f'["{PLATFORM}"]', f'["{PLATFORM}", "linux-arm64"]'),
            encoding="utf-8",
        )

        result = self.run_installer(platform="linux-arm64")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("demo has no sha256 for platform linux-arm64", result.stderr)

    def test_rerun_skips_installed_tool_without_downloading(self) -> None:
        self.archive_tool("demo", "1.2.3")
        self.assertEqual(self.run_installer().returncode, 0)
        for download in self.downloads.iterdir():
            download.unlink()

        result = self.run_installer()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ok demo 1.2.3 (already installed)", result.stdout)
        self.assertEqual(self.installed_output("demo"), "fake 1.2.3\n")

    def test_rerun_replaces_tool_after_pin_change(self) -> None:
        self.archive_tool("demo", "1.2.3")
        self.assertEqual(self.run_installer().returncode, 0)
        self.archive_tool("demo", "1.2.4")

        result = self.run_installer()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("installed demo 1.2.4", result.stdout)
        self.assertEqual(self.installed_output("demo"), "fake 1.2.4\n")

    def test_only_installs_named_tools_and_rejects_unknown_names(self) -> None:
        first_url, first_sha = self.publish("one", fake_tool("1"))
        second_url, second_sha = self.publish("two", fake_tool("2"))
        self.write_manifest(
            raw_tool("one", "1", first_url, first_sha)
            + raw_tool("two", "2", second_url, second_sha)
        )

        result = self.run_installer("--only", "two")
        unknown = self.run_installer("--only", "three")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.tools_dir / "bin" / "two").exists())
        self.assertFalse((self.tools_dir / "bin" / "one").exists())
        self.assertNotEqual(unknown.returncode, 0)
        self.assertIn("unknown tool(s): three", unknown.stderr)


class ToolsHelperTest(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.repo)
        (self.repo / "deploy").mkdir()
        (self.repo / "deploy" / "tools.toml").write_text(
            'schema = 1\nplatforms = []\n[tools.demo]\nversion = "2.0.0"\n', encoding="utf-8"
        )

    def install(self, version: str) -> None:
        for directory in ("bin", "markers"):
            (self.repo / ".tools" / directory).mkdir(parents=True, exist_ok=True)
        binary = self.repo / ".tools" / "bin" / "demo"
        binary.write_bytes(fake_tool(version))
        binary.chmod(0o755)
        (self.repo / ".tools" / "markers" / "demo").write_text(
            f"{version} {'a' * 64}\n", encoding="utf-8"
        )

    def use_tools(self) -> subprocess.CompletedProcess[str]:
        script = f'. "{HELPER}"; baukit_use_tools "{self.repo}" demo; command -v demo; demo'
        return run_process(["sh", "-c", script], capture_output=True, text=True, check=False)

    def test_missing_tool_exits_2_with_install_command(self) -> None:
        result = self.use_tools()

        self.assertEqual(result.returncode, 2)
        self.assertIn("Run: scripts/install-deploy-tools.sh --only demo", result.stderr)

    def test_outdated_tool_exits_2(self) -> None:
        self.install("1.0.0")

        result = self.use_tools()

        self.assertEqual(result.returncode, 2)
        self.assertIn("missing or outdated", result.stderr)

    def test_pinned_tool_comes_first_on_path(self) -> None:
        self.install("2.0.0")

        result = self.use_tools()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.splitlines(), [str(self.repo / ".tools" / "bin" / "demo"), "fake 2.0.0"]
        )

    def test_tool_version_prints_pin_and_rejects_unknown_tool(self) -> None:
        def version(name: str) -> subprocess.CompletedProcess[str]:
            script = f'. "{HELPER}"; baukit_tool_version "{self.repo}" {name}'
            return run_process(["sh", "-c", script], capture_output=True, text=True)

        self.assertEqual(version("demo").stdout, "2.0.0\n")
        unknown = version("missing")
        self.assertNotEqual(unknown.returncode, 0)
        self.assertIn("pins no tool named missing", unknown.stderr)


class ManifestTest(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_lists_every_validator_for_every_platform(self) -> None:
        tools = self.manifest["tools"]
        self.assertEqual(set(tools), REQUIRED_TOOLS)
        self.assertTrue(set(self.manifest["platforms"]) >= REQUIRED_PLATFORMS)
        for name, tool in tools.items():
            with self.subTest(tool=name):
                self.assertRegex(tool["version"], r"^[0-9]+\.[0-9]+\.[0-9]+$")
                if "requirements" in tool:
                    self.assertTrue(all("--hash=sha256:" in line for line in tool["requirements"]))
                    continue
                self.assertEqual(set(tool["sha256"]), set(self.manifest["platforms"]))
                self.assertTrue(all(SHA256.fullmatch(value) for value in tool["sha256"].values()))

    def test_images_are_pinned_by_digest(self) -> None:
        for name, image in self.manifest["images"].items():
            with self.subTest(image=name):
                self.assertRegex(image, r"^[a-z0-9./-]+:[0-9A-Za-z._-]+@sha256:[0-9a-f]{64}$")


class WorkflowCoherenceTest(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_ci_workflows_match_pinned_versions(self) -> None:
        workflows = {
            path.name: path.read_text(encoding="utf-8") for path in sorted(WORKFLOWS.glob("*.yml"))
        }

        self.assertEqual(workflow_version_conflicts(self.manifest, workflows), [])

    def test_detects_conflicting_workflow_pins(self) -> None:
        workflow = """
jobs:
  platform:
    env:
      KUBECONFORM_VERSION: v0.6.0
    steps:
      - uses: azure/setup-helm@abc # v4
        with:
          version: v3.0.0
      - run: curl -LO https://github.com/kyverno/kyverno/releases/download/v1.19.0/kyverno-cli_v1.19.0_linux_x86_64.tar.gz
      - run: pip install yamllint==1.30.0
"""

        conflicts = workflow_version_conflicts(self.manifest, {"ci.yml": workflow})

        self.assertEqual(
            sorted(conflict.split(":")[1].split()[0] for conflict in conflicts),
            ["helm", "kubeconform", "kyverno", "yamllint"],
        )

    def test_accepts_matching_workflow_pins(self) -> None:
        tools = self.manifest["tools"]
        workflow = (
            f"env:\n  KUBECONFORM_VERSION: v{tools['kubeconform']['version']}\n"
            "steps:\n  - uses: azure/setup-helm@abc\n    with:\n"
            f"      version: v{tools['helm']['version']}\n"
            "  - run: scripts/install-deploy-tools.sh --only promtool\n"
        )

        self.assertEqual(workflow_version_conflicts(self.manifest, {"ci.yml": workflow}), [])


if __name__ == "__main__":
    unittest.main()
