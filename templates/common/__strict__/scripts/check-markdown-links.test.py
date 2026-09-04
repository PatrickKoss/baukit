#!/usr/bin/env python3

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SOURCE_SCRIPT = Path(__file__).with_name("check-markdown-links.py")


class MarkdownLinkCheckTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.script = scripts / SOURCE_SCRIPT.name
        shutil.copyfile(SOURCE_SCRIPT, self.script)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "config", "user.email", "links@example.invalid"],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(self.root), "config", "user.name", "Link tests"],
            check=True,
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write(self, relative: str, source: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")

    def add_all(self) -> None:
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "commit", "-q", "-m", "add files"],
            check=True,
        )

    def run_check(self, *roots: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.script), *roots],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_accepts_inline_reference_encoded_and_repository_links(self) -> None:
        self.write(
            "README.md",
            "[guide](docs/guide.md) [web](https://example.com/x) [section](#one)\n"
            "[root](/docs/guide.md) [reference][details]\n"
            "[details]: <docs/file%20name.md#part>\n",
        )
        self.write("CLAUDE.md", "See [guide](docs/guide.md?plain=1).\n")
        self.write("AGENTS.md", "See [guide](docs/guide.md).\n")
        self.write("docs/guide.md", "![image](image.png)\n")
        self.write("docs/file name.md", "# Part\n")
        self.write("docs/image.png", "not an image, but an existing target\n")
        self.add_all()

        completed = self.run_check()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "All local Markdown links resolve.\n")

    def test_reports_source_line_and_missing_target(self) -> None:
        self.write("README.md", "first line\n[missing](docs/absent.md)\n")
        self.add_all()

        completed = self.run_check("README.md")

        self.assertEqual(completed.returncode, 1)
        self.assertIn("README.md:2 -> docs/absent.md", completed.stderr)

    def test_ignores_untracked_markdown_and_unconfigured_roots(self) -> None:
        self.write("README.md", "[guide](docs/guide.md)\n")
        self.write("docs/guide.md", "# Guide\n")
        self.add_all()
        self.write("docs/untracked.md", "[missing](absent.md)\n")
        self.write("notes/tracked.md", "[missing](absent.md)\n")
        subprocess.run(
            ["git", "-C", str(self.root), "add", "notes/tracked.md"], check=True
        )

        completed = self.run_check("README.md", "docs")

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_checks_present_markdown_before_first_git_commit(self) -> None:
        self.write("README.md", "[missing](docs/absent.md)\n")

        completed = self.run_check("README.md")

        self.assertEqual(completed.returncode, 1)
        self.assertIn("README.md:1 -> docs/absent.md", completed.stderr)


if __name__ == "__main__":
    unittest.main()
