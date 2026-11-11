from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from cli_install import PREFIX, ROOT, SUFFIX, instruction_files, problems, update_tags
from process_runner import run_process


class CliInstallTest(unittest.TestCase):
    def test_all_repository_instructions_use_the_current_git_tag(self):
        version = (ROOT / "templates/VERSION").read_text().strip()
        for path in instruction_files(ROOT):
            with self.subTest(path=path.relative_to(ROOT)):
                self.assertEqual(problems(path.read_text(), version), [])

    def test_rejects_stale_tags_and_registry_installs(self):
        for command in [
            PREFIX + "0.1.2" + SUFFIX,
            "cargo install baukit-cli",
            "cargo install --locked baukit-cli",
            "cargo install baukit",
            "cargo install --path cli --locked",
        ]:
            with self.subTest(command=command):
                self.assertTrue(problems(command, "0.7.0"))
        self.assertEqual(problems(PREFIX + "0.7.0" + SUFFIX, "0.7.0"), [])

    def test_release_update_changes_concrete_tags_and_preserves_template_tags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_process(["git", "init", "--quiet", str(root)], check=True)
            readme = root / "README.md"
            readme.write_text(PREFIX + "0.7.0" + SUFFIX + "\n")
            template = root / "README.md.jinja"
            template.write_text(PREFIX + "{{ context.template_version }}" + SUFFIX + "\n")
            run_process(["git", "add", "README.md", "README.md.jinja"], cwd=root, check=True)
            update_tags(root, "0.7.1")
            self.assertEqual(readme.read_text(), PREFIX + "0.7.1" + SUFFIX + "\n")
            self.assertIn("{{ context.template_version }}", template.read_text())

    def test_release_update_skips_tracked_files_deleted_by_version_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_process(["git", "init", "--quiet", str(root)], check=True)
            readme = root / "README.md"
            readme.write_text(PREFIX + "0.7.0" + SUFFIX + "\n")
            changeset = root / "changeset.md"
            changeset.write_text("consumed\n")
            run_process(["git", "add", "README.md", "changeset.md"], cwd=root, check=True)
            changeset.unlink()
            update_tags(root, "0.7.1")
            self.assertEqual(readme.read_text(), PREFIX + "0.7.1" + SUFFIX + "\n")
